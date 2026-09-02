> **Status: planning baseline, 2026-08-30.** Written by the design workflow (3 proposals → 3 judges → synthesis → 2 adversarial critics → revision 2) reviewed by Claude, and walked through with Matt on 2026-08-31 — D35 resolves every open decision, so the baseline stands approved for build. Provenance in `docs/design-review/`, evidence in `docs/research/`. Change decisions through `docs/DECISIONS.md`; keep this file current per arc.

# Roadmap — arcs ≈ sessions ≈ minor versions

Each arc: implement → adversarial review → fix → report → user approves the commit and tag. CHANGELOG entry per arc. Every source feature lands in the arc whose tier first renders it.

## Arc 1 — v0.1.0 "the grid lights up"
*Split 1a (core seam — **Fable**, `docs/briefs/arc-1a.md`) / 1b (cpu + htop + retrowave — **Opus**, `docs/briefs/arc-1b.md`); later arcs get a brief at arc start (D36).*
**Goal:** a screenshot-worthy Overview page in Ptyxis — htop meters plus 32 gradient-coloured core bars in CCD blocks and a big clock, in retrowave and modern — on top of the core seam, with demo mode, the testkit and a measured throughput number. No GPU, no journal, no process table, no effects.
**Deliverables**
- [x] Workspace scaffold: six crates, workspace pins (explicit ratatui-core/-widgets features, crossterm only in app/cli), `rust-version = 1.88`, `clippy.toml` bans on `ratatui::{init,try_init,run,restore}`, `deny.toml` (ISC + CDLA allowed, tokio assertion over four features), `ci.yml` (fmt, clippy -D warnings, test, doc -D warnings, MSRV, per-crate check, feature matrix, deny, tree -d, audit)
- [x] `gridwatch-store`: Ts/Clock, Key/Label/Datum, `RecordValue` + `KeyMeta` catalogue with `lookup()`, Ring (VecDeque), series + retention, `Store::apply` + read API + `resample`, `Msg`/`Batch`/`ControlMsg`/`InputEvent` and the three channels, Source/Sampler/SourceCtx/Demand/Level/Cadence/Control/SourceStatus, `demo::Synth`, catalogue for `sys` and `cpu` (incl. `sensor.temp_c{k10temp:*}`)
- [x] `gridwatch-ui`: Component/Manifest (sources + optional_sources)/ComponentDef/Registry/BuildCx/Command/Action, Footprint/Tier (cumulative, `zoom_only`)/Chrome + `view` resolution + `demand`, `View` tree + `Span` + the default `Renderer` for Text/KeyValue/Gauge/Segmented/Bars/Sparkline/Chart/Table/BigNumber/Stack/Custom parameterised by `Theme::widgets`, view-tree YAML snapshots, `schema/view.schema.json` + `schema/{config,layout,theme}.schema.json` with fixture validation in CI, theme `class` and `[widgets]` parsed (`[ambient]` ignored with one warning until arc 4), layout engine (tracks/thresholds/solve/hit/unit_at/focus_dir, 12×6, size-derived Configured/Dense/Stack with hysteresis, chip fallback, too-small notice; the pure edit ops and their proptests land here — the edit *mode* is arc 4), Theme loader v1 (roles, `$palette`, Oklab gradient LUTs, glyph tiers, borders, title styles, ColorMode ladder + mono; `[flourish]`/`[effects]`/`inherits` parsed and ignored with a warning), built-in `modern`/`retrowave`/`mono`, widgets stacked_bar/vbars/big_number/chip/kv_table/sparkline_ext, overlay (help, stats HUD, too-small), `dump::cells` (RLE styled) + `dump::ansi`, testkit (`snapshot_matrix!`, `role_swatch!`, `assert_never_panics`, `assert_min_tier_fits`, `real_grid_sizes()` for 250×70 configured and 120×40 dense)
- [x] `gridwatch-sources`: supervisor (catch_unwind, backoff, restart counter), `cpu` (procfs: stat breakdown with guest subtraction, meminfo formulas, loadavg, PSI, topology, freq, k10temp Tccd by label; no process table yet)
- [x] `gridwatch-components`: `clock` template (Chrome::Borderless honoured), `sources` tile, `htop` tiers tiny/big-number/meters/cores (read-only)
- [x] `gridwatch-app`: own terminal init/restore, PanicPolicy hook, stderr dup2, input thread with crossterm→InputEvent conversion, `run<B: Backend>`, loop with drain order + per-source generation dirty gating, 30/60 fps, pages/hotkeys, Tab + hjkl focus, Enter/Esc capture, `z` zoom, `d` dense override, `t` theme cycle, `space` pause, Bg painted first, config.toml + layout.toml loading with spans and the options-disjointness check, `F12` HUD, `S` screenshot
- [x] `gridwatch` CLI: `run [--demo [--seed N]|--page|--theme|--fps|--color|--no-mouse|--stats]`, `shot --format ansi`, `config check|default`
- [x] Tests: store unit tests (ring, resample, apply, control-under-full-data), tracks-vs-ratatui oracle, `thresholds()` for the default grid, proptests (mode monotonic with hysteresis, no-panic sweep), snapshot matrix for clock/htop at real sizes (modern) + role swatches for all three themes, default-layout no-chip assertion at 120×40 and 250×70, htop formulas against `fixtures/procfs`
- [x] `scripts/perf/measure.sh` (per-thread pidstat, task-summed voluntary switches, Δwchar from /proc/<pid>/io, nvidia-smi pmon, pw-top) and `--stats-log`; named threads; `EnableFocusChange` → unfocused throttle (`[perf] unfocused_fps = 2`, level → Hidden and detail → Meters); zero-poll `SourceCtx::sleep_until` on `[perf] phase_ms = 250`; render cache keyed by source generations; frame coalescing
- [x] `docs/PERFORMANCE.md` first measured rows: p50/p95 frame time, changed cells, bytes/frame and wake-ups/s at 250×70; 30-second Ptyxis glyph check (rounded corners, eighth blocks, braille, octant) recorded in `docs/THEMES.md`; `docs/PARITY.md` htop section; README with an ANSI dump and a captured PNG
**Acceptance:** `cargo test --workspace` passes with no hardware; `gridwatch --demo` and `gridwatch` (live) show the Overview page (htop meters + cores + clock + the `sources` tile in the `amp` slot; other placements as placeholder chips) in both themes at 250×70 with p95 frame time <8 ms; at 120×40 the page renders in dense mode with every tile above chip level; clippy/fmt/doc/MSRV/deny/per-crate check green.
**Performance gate:** `scripts/perf/measure.sh` exists and P1, P4, P5, P6, P8, P18, P19, P21 are green on torch (first measured rows land in `docs/PERFORMANCE.md`); VTE focus reporting verified so the unfocused throttle works. **Read the status line below before treating this row as met — P4 and P21 are owed against the 1b build.**
**Status 2026-09-01 (arc 1 committed and pushed, CI green; `v0.1.0` untagged):** every deliverable above is implemented and gated. P1, P5, P6 (with the HUD-vs-Δ`wchar` cross-check at 1.99 %), P8, P17, P18 and P19 are measured green under a 250×70 pty on an idle box; **P4, P21 and the Ptyxis rows P9/P10 are owed** — a pty sends no focus events — as is a re-take at Matt's real window size with the game running. The Ptyxis glyph check and the README PNG are the two rows a machine cannot tick. Two startup defects found by running the binary by hand were fixed after the review: a non-tty exited silently (stderr was redirected before the terminal opened) and a too-small terminal drew a blank screen (§6's notice existed but nothing called it).
**Risks:** VTE throughput below expectation (the HUD decides whether 60 fps stays opt-in); the trimmed theme loader tempting scope creep back in (inherits/overrides are explicitly arc 3).

## Arc 2 — v0.2.0 "history and the GPU"
*Brief: `docs/briefs/arc-2.md` (D47, 2026-09-01) — two Opus sessions, **2a** journal + pid scan + htop table + tooling, then **2b** the GPU.*
**Goal:** the GPU component with nvtop's rolling charts, the process table, and journal record/replay with a determinism test and CI screenshots.
**Deliverables**
- [ ] Journal: RecordValue to_json/decode, Recorder tee, Replay + JournalSource, virtual clock, round-trip test over every catalogued Record type, `--record F`, `--replay F --speed N`, `--record-input`
- [ ] `gpu` source: NVML static probe, fast/slow tiers, power trace, PCIe byte counters, NotSupported pruning, LibRmVersionMismatch/Libloading states, nvidia-smi fallback tier, const spec table cross-check; keys labelled `{0}`; Synth GPU data
- [ ] `gpu` component tiers badge/gauges/header/charts/**procs** (mins 8×3 / 24×5 / 56×8 / 56×12 / 56×18; resample-driven 10-minute charts in a 4–8-row band, selectable series, reverse, effective-load, spec column; `procs` = top-N GPU process table with the grid default `PID DEV TYPE GPU GPU MEM CPU HOST MEM Command` (`USER`, `ENC`, `DEC` via `columns`), default sort `gpu_mem`, `table_rows = 10`, zoom fills the body; select, `</>`/`F6` sort and `I` only — `/`, `F9` and Actions are arc 8); manifest `optional_sources = [cpu]`, `demand(procs) = Detail::Table`
- [ ] `gpu` source process rows: v3 graphics + compute lists (`InsufficientSize` retried) overlaid with `process_utilization_stats(last_seen)` using nvtop's carry-forward timestamp, gated on `Detail::Table`; fans %/RPM at 5 s and `samples(Power)` only at `header`+ so the slow tier stays ≈ 4.3 ms/s with process rows; `gpu.procs{dev}` key; joined with `proc.table` in the component's `tick` with a per-PID last-known cache
- [ ] `cpu` source pid-level process scan at `Detail::Table` (`stat`, `statm`, dir `st_uid`, `cmdline` on first sight; deltas keyed by `(pid, starttime)`; `sys.pid_digits`; 3 s grid / 1.5 s focused); htop `table` tier (min 56×18; top-N with the grid default `PID RES SHR S CPU% MEM% TIME+ Command` — htop's full Main screen `PID USER PRI NI VIRT RES SHR S CPU% MEM% TIME+ Command` via `columns` and in `full` — at htop's printed widths incl. auto-width CPU%, `Row_printKBytes` regimes, state colours, default sort `cpu`, `hide_userland_threads = true`, `table_rows = 10`, gridwatch's drop order with `command_min = 20`, zoom fills the body); §8.1 snapshot tests at 122×31, 80×20, 59×18 and 248×66
- [ ] `demo::Synth` 32-process `proc.table` + `gpu.procs` set (a game at 12.5 GiB / 17 % SM, a shell, a browser, kernel threads, one `Both G+C` process, VIRT/RES values hitting all three KiB regimes)
- [ ] `dump::svg` + `shot --format svg`; CI screenshot job regenerating `docs/img/*.svg` and failing on drift; `component list|info`, `keys` → `docs/COMPONENTS.md`, `docs/KEYS.md`
- [ ] Fixtures `torch-idle.jsonl`, `torch-game.jsonl`; determinism test replaying through `run<TestBackend>` twice with identical frame hashes; `docs/PARITY.md` nvtop section
**Acceptance:** gpu header/gauge and process numbers match `nvtop --snapshot` within a tick on torch; the Overview at 250×70 shows 10-row tables in both 6x3 tiles, the 120×40 dense layout shows 5-row tables, and the zoomed gpu tile shows CPU/HOST MEM/Command for the game; charts show 10 minutes of history; replay of a fixture is byte-identical across two runs; README images come from CI.
**Performance gate:** P11 (≤ 6 ms/s with the Overview's `procs` tile visible, per-class sum shown in the `sources` tile), P12, P13, P15, P17 (1 h) green per `docs/PERFORMANCE.md`; the process rows add ≈ 2 ms/s and nothing when no gpu tile is at `procs`.
**Risks:** NVML field quirks on driver 610 (per-field probing); journal size with tables (tables off by default).

## Arc 3 — v0.3.0 "the signature tile"
**Goal:** astral-watch parity as a first-class component with the cross-page alarm overlay, plus the degraded-mode story.
**Deliverables**
- [ ] `pins` source (git-pinned astral-watch, auto exporter → i2c; Lifecycle bridge with self-tracked active set, TelemetryLost feed, redetect); `pins` keys; Synth pin data with scripted overload; CSV tail deferred to arc 8
- [ ] `pins` component tiers watts-badge/mini-bars/bars/trend/full (single card; tui.rs parity incl. device header from `gpu.info` + sysfs PCIe link, braille trend chart, log with scrollbar, pause/reset/rate keys)
- [ ] AlertLog + control-channel alerts + overlay banner/toasts/ack + `alerts` tile; blink-free alert pulse (tachyonfx comes in arc 4)
- [ ] **Record the alerting stance (DECISIONS entry):** gridwatch is a viewer — the astral-watch service owns out-of-band alerting; the optional unfocused `notify-send` on Crit stays in BACKLOG unless promoted here (D39)
- [ ] Capability probe (`CapSet`, ≤200 ms), `Manifest.requires/optional`, placeholder tiles with fix text, `gridwatch doctor`, staleness dimming + `STALE` badge
- [ ] 1 Hz mtime hot reload for config/layout/theme with instance diffing and error toasts; theme loader v2 (`inherits`, `[components.<kind>]` overrides, WCAG warn gate); `terminal` and `phosphor-green` themes
- [ ] **Pre-req, confirmed 2026-08-31 (D35):** in a separate astral-watch session, open the three PRs (`cli`/`notify` feature gating, `log` facade, `Lifecycle::active()`) and cut the v0.8.0 tag; gridwatch then pins the tag instead of the rev. **Decide crates.io in that same session** — a git dependency can never be published, so if gridwatch is ever to reach crates.io, `cargo publish` astral-watch 0.8.0 then (D39)
- [ ] Fixtures: `synth-overload.jsonl`; replay test asserts `pins/overload` raised at the scripted Ts and the banner renders on page 2; `docs/PARITY.md` astral-watch section
**Performance gate:** P14 green (≤ 2 i2c transactions/s, ≤ 1 % CPU); the banner adds no steady-state redraws.
**Acceptance:** pins tile matches astral-watch's TUI feature list per PARITY.md; a synthetic overload raises the red banner on every page and resolves with a green toast; running beside the root logger shows no corrupted readings; `gridwatch doctor` lists every capability with reasons.
**Risks:** astral-watch API drift before the tag (pin rev); i2c contention latency (≥500 ms, exporter preferred); GPU idle zeros (TelemetryLost path tested via replay).

## Arc 4 — v0.4.0 "rearrange and shine"
**Goal:** edit mode, the effects layer, and the `matrix` theme as the first showcase-class theme. Realistically two sessions: **4a** edit mode, **4b** effects + matrix. Matrix's theme-vs-usability balance is explicitly expected to iterate here (Matt, 2026-08-31) — budget review time for tuning `fade_s`/`trail_ms`/`sweep_s`/density against real use, and treat D31/D34's numbers as starting points, not commitments.
**Deliverables**
- [ ] Edit mode state machine over pure page ops: HJKL move, Ctrl-hjkl resize, `s` footprint cycle, `S` swap, `a` picker (instances or `kind:` shorthand, first-fit insert), `x` remove, undo/redo, red-ghost collisions, dotted unit grid, mouse drag/corner resize
- [ ] `w` save via toml_edit into `layout.toml` only (comments preserved, atomic rename, re-parse check, self-write hash)
- [ ] tachyonfx effect hooks (startup sweep, theme_swap fade, focus fade, alert hsl pulse, optional CRT ambient off by default) with `budget_ms` watchdog and `--no-effects`; `[effects]` now consumed
- [ ] Flourishes: gradient titles, sun + grid floor in empty slots, big clock pixel modes; `phosphor-amber`; WCAG `contrast.autofix`
- [ ] **`matrix` theme (D28):** `class = "showcase"`, the ambient layer as a post-render tachyonfx `effect_fn` with a veil mask, `matrix_rain` (seeded droplets, head/trail through the `Rain` gradient, glyph mutation, half-width katakana with `rain = "ascii"` fallback), the **rain-lit renderer** (D31: per-cell `lit` brightness; a droplet head passing a content cell shows the rain glyph for one frame then the module's own character at full brightness, which fades through its own colour to the floor over `fade_s`; a dense **sweep** every `sweep_s` re-renders the whole page in one fall; `relight_on_update` re-lights changed cells so live values stay bright), readability pins (focused tile, alerting tiles, banner/toasts, key bar, hover and `reveal_ms` after a key; `V` re-lights the page, `L` locks all lit), `rain_fill` startup, freeze on `FocusLost`/pause, the governor (fps → density → sweep period → gutters-only, 30 s recovery, HUD state), determinism + readability + fade + sweep + re-light tests, a 30-second Ptyxis glyph check that katakana render one cell wide (Noto Sans Mono CJK JP fallback)
- [ ] Edit-op proptests extended to the mouse drag / corner-resize paths (the core op proptests landed in arc 1)
**Acceptance:** a layout edited in-app round-trips through `layout.toml` with comments intact and `config.toml` untouched; effects stay within budget on the HUD; retrowave vs modern screenshots regenerated in CI; under `matrix` the rain runs at 24 fps on the Overview with the game running, the whole page fades between sweeps and is re-rendered by each fall, a GPU-utilisation change re-lights exactly its cells, a synthetic pins overload is readable through it, tabbing to the game drops gridwatch to P4 within one frame, and the governor engages when the window is stretched to a size the terminal cannot sustain.
**Performance gate:** P20; idle edit mode meets P8; S1–S7 under `matrix` focused, S5 = P4 with the game focused.
**Risks:** effect cost on VTE (area scoping + watchdog); toml_edit formatting loss on removed items (documented).

## Arc 5 — v0.5.0 "it moves"
**Goal:** the audio visualizer and the sensors tile.
**Deliverables**
- [ ] `audio` source: pw-record supervisor (latency 1024 default, `low_latency` opt-in, EOF-only respawn, passive-silence rule, kill after 10 s hidden), rtrb ring, DSP thread (dual FFT 8192/2048, 64 log bands, floor/tilt, scope, RMS/peak, 2 Hz idle publishing), `pw-dump` sink enumeration, Synth stereo mix
- [ ] `audio` component: ballistics presets (winamp/cava), tiers vu/mini/scope/spectrum/full, octant-or-braille scope, global sink picker, `Animated{fps}` while visible with `fps = 30` default
- [ ] `sensors` source (hwmon walker, PSI, cpufreq, RAPL gate; takes over `sensor.temp_c{k10temp:*}` from cpu) and component tiers hottest/strip/table/chart/full
- [ ] DSP tests (0 dBFS sine, 50 Hz single-band); record `fixtures/journals/torch-audio.jsonl`; hardware-gated `#[ignore]` capture test; HUD measurement of the animated cell at 30 and 60 fps
**Acceptance:** live spectrum reacts to Firefox/game audio within ~30 ms; CPU <5 % of one core with the visualizer at 30 fps and <8 % at 60; `pw-top -b -n 1` shows the graph quantum unchanged at the default latency with no game running and ≤512 only with `low_latency` while the widget is visible.
**Performance gate:** P2, P3, P7, P9, P10, P16 — including the Ptyxis CPU/SM delta with the visualizer visible beside the game.
**Risks:** pw-record flag drift across PipeWire versions (version check + `--help` parse); eighth-block coverage in the user's actual font (ascii tier fallback).

## Arc 6 — v0.6.0 "Winamp"
**Goal:** the MPRIS now-playing component in classic-skin form.
**Deliverables**
- [ ] `mpris` async source on the private tokio thread: proxies, discovery, per-player tasks, position model, art fetch/decode, `MediaCmd` control, Synth fake player with embedded PNG
- [ ] `winamp` component: Custom chrome, `optional_sources = [audio]` with static vis fallback, tiers status/shade/main/main+art/full, marquee from `cx.now`, big digits, 19-band vis from `audio.bands`, kbps/kHz from `audio.sink`, posbar/volume/transport, EQ weighting, local playlist history, halfblock art
- [ ] Recorded `a{sv}` metadata fixtures (Firefox/YouTube, a single-element page) for decoding tests
**Acceptance:** Firefox playback controls work from the tile; track changes update within one poll; stream mode when `mpris:length` is absent; replay determinism holds with the marquee running; the tile renders on a page without the audio component.
**Risks:** Firefox Position=0 on multi-element pages (stream mode); zbus/tokio feature interplay (asserted by the deny job).

## Arc 7 — v0.7.0 "wired"
**Goal:** the network component (Tier 0) and generic alert rules.
**Deliverables**
- [ ] `net` source: all-interface rates/link/addrs/route/DNS, connection table at `Detail::Table`, `net-probe` (ICMP DGRAM → TCP fallback) with ring stats, optional neli-wifi and NM fallback, public IP opt-in
- [ ] `net` component tiers rates/sparks/table/conns/full with instance-level interface filters and rDNS opt-in
- [ ] `[[rules]]` engine (name-indexed, hold/clear, metric RHS, label wildcards, absent) + `config check` for rules; default rules for gpu-hot, cpu-hot, nvme-crit, link-down
- [ ] Capability badge and design note for the future `gridwatch-netd` helper (per-process bandwidth)
**Acceptance:** rates match `ip -s link` deltas; probes show gateway ~1–4 ms; own-process connections attributed; a rule raises and clears with hysteresis in replay.
**Risks:** Wi-Fi path untestable until wlp7s0 is connected (fixtures); mDNS stalls (async resolver only).

## Arc 8 — v0.8.0 "full parity"
**Goal:** htop and nvtop interactive parity and the remaining pins modes, checked against PARITY.md.
**Deliverables**
- [ ] htop full tier: screens/tabs, tree view, search/filter, sort keys, tags, follow, kernel/user thread toggles, F-key bar; actions kill/renice/affinity/ioprio via `Action`s with confirm + `readonly`
- [ ] gpu `full` tier (zoom-only): `F6` sort by any nvtop criterion, `+`/`−`, `F9` signal menu via `Action`, Power sub-panel showing pins under board power; htop `full`: I/O screen columns and htop's gated files at `Detail::Columns`, `H` userland threads with the `task/` walk, `/` search, tree, filter, tags; the executor thread and every `Action` land here
- [ ] pins CSV tail mode verified against a running service; exporter mode verified; multi-card tabs
- [ ] `exec` plugin host — the public plugin API: JSON-lines protocol (hello/manifest, samples, render → view tree, keys, commands, status) validated by `schema/exec.schema.json` + `schema/manifest.schema.json`, supervised like a source, an example Python plugin under `plugins/examples/`, `docs/PLUGINS.md`; a plugin may be a source, a component, or both; **security posture** (D39): schema-validated input only, max line length, unknown kinds rejected, CPU/RSS caps with kill-on-runaway, no shell interpretation of plugin commands
- [ ] `docs/PARITY.md` diff: every in-scope row checked
**Acceptance:** htop muscle memory transfers for the listed keys; EPERM surfaces as a toast; nvtop header/process numbers match `nvtop --snapshot` within a tick; no in-scope PARITY.md row unchecked.
**Performance gate:** zoomed `full` tiers stay inside P15 (`smaps_rollup` only with the column on) and P19.
**Risks:** privilege edge cases (documented, never escalated).

## Arc 9 — v0.9.0 "ship it"
**Goal:** packaging, docs and 1.0 readiness.
**Deliverables**
- [ ] `release.yml` gnu + musl tarballs, nfpm deb/rpm, AUR PKGBUILD, Nix flake, udev RAPL rule (optional)
- [ ] `gridwatch theme import` (alacritty/wezterm/base16)
- [ ] Docs complete (ARCHITECTURE, ADDING-A-COMPONENT, COMPONENTS, THEMES, LAYOUT, KEYBINDINGS, PERFORMANCE, PARITY, CONTRIBUTING), README with per-theme screenshots, THIRD_PARTY.md
- [ ] crates.io publication path decided (blocked on astral-watch 0.8.0), `cargo install --git` documented
- [ ] Bench suite (criterion) for apply/resample/full-frame render committed with baseline numbers
**Acceptance:** a fresh Ubuntu container with only build-essential + pkg-config builds every feature; packaged binary runs on torch with all seven components live.
**Risks:** musl + NVML dlopen path (runtime test in release job).
