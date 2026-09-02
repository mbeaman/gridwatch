> **Status: planning baseline, 2026-08-30.** Written by the design workflow (3 proposals → 3 judges → synthesis → 2 adversarial critics → revision 2) reviewed by Claude, and walked through with Matt on 2026-08-31 — D35 resolves every open decision, so the baseline stands approved for build. Provenance in `docs/design-review/`, evidence in `docs/research/`. Change decisions through `docs/DECISIONS.md`; keep this file current per arc.

# Decision log

One entry per decision, with the reason. Append; do not rewrite history — supersede with a new entry that links the old one.

## D01 — Base proposal and grafts

**Decision.** Start from the contract-first design (Manifest/ComponentDef/Registry, cumulative tiers with minimum sizes, capability probe, effects as data, 12-column grid, shell-drawn chrome) and graft the single-writer typed Store, virtual Clock, journal replay, demand levels and generation dirty gating from the store-first proposal, plus deny.toml, the 1 Hz mtime watcher, the halfblock painter, the stats HUD and a screenshot-first arc 1 from the pragmatic proposal.

**Why.** Two judges scored the contract-first design highest and it is the only one whose 1x1..full-screen story survives measurement on this terminal; the store-first data model removes the Feed/ArcSwap ordering hazard and the two homes for data; the pragmatic tooling choices are the cheapest correct ones.

## D02 — Crate structure

**Decision.** Six workspace crates: gridwatch-store (no TUI crate, no crossterm, no system deps; also hosts demo::Synth and InputEvent), gridwatch-ui (ratatui-core/-widgets with explicit std/serde/underline-color/layout-cache features, testkit feature), gridwatch-sources and gridwatch-components (feature-gated modules), gridwatch-app (shell as a library, generic over Backend), gridwatch (binary). Direction store ← ui ← components ← app ← bin and store ← sources ← app; crossterm only in app and bin.

**Why.** Keeps enforced boundaries without a 19-crate ceremony; the store compiles in about a second; moving Synth into the store lets the ui testkit and --demo share one generator without breaking the dependency direction (review finding); ratatui-core's default = [] and ratatui-widgets' calendar default were verified, so features must be explicit for per-crate builds.

## D03 — Data model and record serialisation

**Decision.** One Store owned by the render thread, mutated only by Store::apply(&Msg); typed Key<T> catalogue with Scalar (history), Vector (short history) and Record (latest) data; Datum::Record holds Arc<dyn RecordValue> (blanket impl over Serialize types with to_json/as_any) and every Record type registers a decode fn in KeyMeta; metric and source names are interned through the static catalogue on journal read; Store::resample is the only history API; GPU keys carry a device label from day one.

**Why.** The reviewed plan's Arc<dyn Any> records and &'static str ids could not be journaled at all, which broke replay, fixtures and headless screenshots (major finding); a serialisable record trait plus a catalogue-driven decoder fixes it without leaking names; labelling GPU keys now avoids a breaking catalogue change for multi-GPU.

## D04 — Channels and control-plane reliability

**Decision.** Three channels: input (unbounded, drained first), control (unbounded: Status, Alert, Done, Reload; drained second; blocking send from sources), data (bounded 4096, try_send with drop counters, drained for at most 3 ms). Msg is assembled by the loop from all three and teed to the recorder.

**Why.** The single lossy try_send channel shared with 60 Hz audio batches could silently drop the very Overload alert the signature feature exists to deliver (major finding); separating control from data makes alarms and statuses unloseable while keeping telemetry backpressure-free, and gets crossterm input out of the store.

## D05 — Input types

**Decision.** gridwatch_store::InputEvent (Key/Mouse/Resize/Paste/Focus mirror with serde) converted once in the app's input thread; Component::on_key/on_mouse take the mirror types; crossterm never appears below gridwatch-app.

**Why.** Two findings: Msg::Input(crossterm::Event) and on_key(crossterm::KeyEvent) put crossterm into crates claimed to be headless and were missing from the dependency table; a mirror type also makes --record-input trivially serialisable.

## D06 — Threading

**Decision.** Std threads: one input thread (sole event::read caller), one thread per blocking source, one executor thread for Actions, one 1 Hz watcher, and the render thread owning Terminal/Store/App/effects; async-native crates (zbus, surge-ping, hickory) run on a single current_thread tokio runtime inside gridwatch-sources behind the mpris/net-probe/net-rdns/net-dns features; cargo-deny asserts tokio is absent unless one of those four is enabled.

**Why.** Every collector but D-Bus/ICMP/DNS is a blocking syscall; tachyonfx effects are !Send; the ratatui FAQ recommends this loop; the tokio assertion was rephrased because net-rdns/net-dns alone also require tokio (minor finding).

## D07 — Component contract and tiers

**Decision.** Object-safe Component with tick(&mut self) for derived/animation state, render(&self) pure over store+theme+now, tiers() poorest-first and cumulative (each tier draws the previous one plus its `adds`), tiers[0].min ≤ the grid's min_unit_inner (enforced by assert_min_tier_fits); a placement's `view` is a preferred tier honoured when its min fits, otherwise the richest fitting tier with a `view↓` chip; SizeClass, Shape and footprint are removed from the render context; Manifest gains optional_sources and a Chrome mode honoured from arc 1.

**Why.** Reviewers showed the alternative-tier reading lost the CCD core blocks at 6x3 and left `view` undefined when it did not fit (major finding); cumulative tiers match the research footprints; SizeClass only steered titles and was dropped rather than kept as ceremony; optional_sources lets Winamp use audio bands and lets Demand see it (major finding).

## D08 — Side effects

**Decision.** Command enum for shell actions plus Command::Run(Box<dyn Action>) and Control::Domain(Box<dyn Any + Send>) for open-ended component/source actions, executed on the executor thread and Debug-printable for tests.

**Why.** Avoids a closed Command enum that every component arc would have to edit while keeping 'keys in, commands out' testability and never blocking a frame.

## D09 — Terminal lifecycle and panic containment

**Decision.** Never call ratatui::init/try_init/run/restore (clippy.toml disallowed-methods); the app does enable_raw_mode + EnterAlternateScreen/EnableMouseCapture/EnableFocusChange + Terminal::new(CrosstermBackend) and the mirror on exit; run<B: Backend> so TestBackend drives the whole app; one panic hook consults a thread-local PanicPolicy (source threads and contained component calls unwind into catch_unwind; anything else restores the terminal and defers to color-eyre).

**Why.** ratatui 0.30.2's set_panic_hook restores the terminal before the chained hook, which would wreck containment; the previous plan cited astral-watch's tui.rs as the reference, but that file calls ratatui::init() (minor finding), so the sequence is now spelled out and enforced by clippy.

## D10 — Grid geometry and degradation

**Decision.** 12 columns × 6 rows default (per-page override, 24 allowed), rows always explicit, gap 1, min_unit_inner 8×3; the solve mode is derived from terminal size: configured needs 131×37 for the defaults, dense (gap 0, shared borders, short titles) needs 109×27, stack below; 2-cell hysteresis on the way back up; a starved cell never changes the mode.

**Why.** The constant min_terminal 100×30 contradicted tier minimums and the ladder never triggered where it mattered (major finding); deriving thresholds from the spec guarantees every placement has inner ≥ 8×3 above stack mode (120×40 dense gives 1x1 = 9×5, 4x2 = 39×11). The reviewer's rows = "auto" default is rejected: placements are in fixed row units, so a changing row count would push them out of bounds; the auto-rows formula is removed instead.

## D11 — Theme system

**Decision.** Semantic roles + Oklab 64-entry gradient LUTs + glyph tiers + border/title specs + declarative flourish/effect hooks in TOML; colour mode resolved once (CLI > config > NO_COLOR → mono theme > COLORTERM > TERM); Bg painted first; rounded corners force BorderMode::Each; arrays over #[repr(u8)] enums; in-tree nearest-256 mapper. The loader is staged: arc 1 = roles/$palette/gradients/glyphs/borders/titles with modern, retrowave and mono; arc 3 = inherits, [components.<kind>] overrides, WCAG warn; arc 4 = flourishes, effects, autofix; unknown/unused tables are parsed and ignored with one warning meanwhile.

**Why.** Components never name colours so one render path yields retrowave and modern; crossterm 0.29 drops all colour under NO_COLOR; enum-map 3.x requires Rust 1.95 and ansi_colours is LGPL; staging the loader is how arc 1 is halved without losing the retrowave look; ignoring [effects] with a warning removes the silent no-op the reviewer flagged.

## D12 — Effects

**Decision.** tachyonfx 0.25.1 only in gridwatch-app, scheduled for arc 4, mapped from theme EffectHooks; event effects ≤600 ms and area-scoped, ambient CRT off by default, budget_ms watchdog, --no-effects.

**Why.** Effects are polish that must not block the first three arcs; full-screen effects can blow the VTE write budget, so they are bounded and measured with the HUD that ships in arc 1.

## D13 — Demand and dirty gating

**Decision.** Per-source Demand { level, detail } atomics written after each cached layout solve from the visible cells' sources ∪ optional_sources; sources declare Cadence per level and always_on (pins); redraw only when a needed source's generation advanced, an animated visible component is due, effects run, or the 1 Hz heartbeat fires; pause stops emission at the source.

**Why.** Keeps the process under 2 % CPU beside a game, gates the 130 ms smaps_rollup cost by column visibility, prevents hidden pages from causing redraws, and stops the pause key from filling the channel with stale batches.

## D14 — Audio capture and DSP

**Decision.** Supervised pw-record subprocess with --latency 1024 by default (low_latency opt-in uses stdbuf -o0 + 256 only while visible) → rtrb SPSC → DSP thread with a cava-style dual FFT (8192 below 250 Hz, 2048 above), 64 log bands per channel published through the data channel at [sources.audio] fps (30 default, 60 opt-in) while input is above the floor and 2 Hz on silence; respawn only on EOF/exit; child killed after 10 s hidden; sink selection is global; components apply ballistics in tick.

**Why.** Verified on torch that --latency 512 would lower the PipeWire graph quantum from its 1024 default whenever no game pins it, while buying no cadence (minor finding); 64 bands from one 2048 FFT put the whole bass range on ~10 bins (minor finding); a 30 fps default keeps the shipped Overview inside the CPU headline (minor finding).

## D15 — astral-watch integration

**Decision.** Pinned git rev dce7eee (tag v0.8.0 when cut), default-features = false, never tui/safety; [patch] to the sibling checkout in a git-ignored .cargo/config.toml; source auto-selects exporter → i2c (≥500 ms) in arc 3 and CSV tail in arc 8; Lifecycle from config::load(None) runs in the source, which tracks the active Condition set itself and emits control-channel alert transitions; stderr dup2'd before the alternate screen; upstream PRs for features, log facade, Lifecycle::active().

**Why.** The crate is not on crates.io and HEAD's API differs from v0.7.0; Lifecycle exposes only new()/observe(); kernel per-adapter locking makes concurrent reads with the root logger safe; the exporter is authoritative when the service runs; library eprintln!s would corrupt the alternate screen; the CSV mode moved to the parity arc to trim arc 3.

## D16 — GPU data and nvtop parity

**Decision.** nvml-wrapper 0.12.1 on its own thread with fast (250 ms) and slow (1 s) tiers, PCIe rates from byte-counter fields 197/198, VRAM and MEMCTL as distinct keys, per-field NotSupported pruning, a hand-verified const spec table, nvidia-smi only on LibloadingError, LibRmVersionMismatch shown without retry; the gpu component ships badge/gauges/header/charts (nvtop's 10-minute resample charts) together in arc 2; process lists and the full tier follow in the parity arc; docs/PARITY.md defines parity per tool.

**Why.** nvtop's core UI is the rolling chart and the previous roadmap deferred it six arcs while building process data nobody rendered (major finding); measured pcie_throughput blocks 21 ms; nvtop's MEM bar and NVML memory utilisation differ; gpuwatch's DB mislabels the 5090.

## D17 — CPU data and the Tccd hand-off

**Decision.** procfs 0.18 with default-features = false and htop's formulas reimplemented; sysinfo not used; the cpu source reads k10temp Tccd temperatures by label in arc 1 and emits sensor.temp_c{k10temp:*}; the sensors source takes the unchanged key over in arc 5.

**Why.** htop parity needs the per-class breakdown and gated files that sysinfo lacks (and sysinfo would raise the MSRV to 1.95); the htop cores tier needed a key whose producer was scheduled four arcs later (major finding), so the producer moves without changing the vocabulary.

## D18 — MPRIS and album art

**Decision.** Hand-rolled zbus 5 proxies (Position uncached) on the private tokio thread; art decoded with image 0.25 on the source thread and painted by the in-tree halfblock widget; Winamp declares audio as an optional source and draws a static vis when bands are absent; no ratatui-image, no Sixel/Kitty.

**Why.** The mpris crate links libdbus (absent); Ptyxis/VTE 0.84 has no Sixel or Kitty graphics and ratatui-image's default feature needs libchafa; the optional-source declaration keeps pw-record alive when only the Winamp tile is visible (major finding).

## D19 — Configuration files

**Decision.** Two files: config.toml (behaviour, [sources.<id>] singletons, [[components]] instances with view-only options, [[rules]]) and layout.toml (grid, pages, placements by id or `kind:` shorthand — the only file edit mode writes); themes/*.toml; a test asserts source option names and component Options names are disjoint per domain; toml 1.1 + serde with spans, hand layering; 1 Hz mtime watcher; toml_edit for saves.

**Why.** The reviewed plan put the same knob in two places, left singleton semantics unstated, and had edit mode rewriting the file that held behaviour options (major finding); the two-file rule now matches its stated purpose; rules fold into config.toml to cut file count.

## D20 — Testing and snapshots

**Decision.** insta snapshots of run-length-encoded styled cell dumps at the modern theme only, per real grid size on 250×70 (configured) and 120×40 (dense); one role-swatch snapshot per theme plus targeted cell assertions; assert_never_panics and assert_min_tier_fits per component; a default-layout no-chip assertion; JSONL journal replay with a determinism test through run<TestBackend> on a virtual clock; seeded demo mode with a headless shot command (ANSI in arc 1, SVG + CI screenshot job in arc 2); hardware-gated #[ignore] tests; PARITY.md as the acceptance oracle for parity arcs.

**Why.** Styled dumps across every theme would produce hundreds of churn-prone files (minor finding); swatches catch palette regressions cheaply; TestBackend's Display drops styles; replay and demo share the source seam; the SVG dumper was unscheduled in the previous plan (minor finding).

## D21 — MSRV and dependency policy

**Decision.** rust-version 1.88 (ratatui 0.30.2 floor), exact workspace pins, cargo-deny allowlist MIT/Apache-2.0/BSD/ISC/CC0/Zlib/Unicode/CDLA-Permissive-2.0 with bans on cpal, mpris, pipewire, libpulse*, ansi_colours, enum-map; cargo tree -d duplicate guard; per-crate cargo check without feature unification; feature-matrix CI proving header-free builds; libc 0.2 line only.

**Why.** ring is Apache-2.0 AND ISC and webpki-roots is CDLA-Permissive-2.0 (verified today), both unavoidable via ureq/rustls under astral-watch, so the previous allowlist failed on day one (minor finding); enum-map 3.1.0 and sysinfo 0.39.6 need 1.95; ratatui-core duplication between widget crates is a real risk.

## D22 — Arc ordering and arc-1 scope

**Decision.** Nine arcs: 1 core + grid + modern/retrowave/mono + cpu meters/cores + clock + HUD (no GPU, no journal, no process table, no effects); 2 gpu with charts + process table + journal/replay + CI screenshots; 3 pins + alerts + capability/doctor + staleness + hot reload + two themes; 4 edit mode + effects + flourishes; 5 audio + sensors; 6 Winamp; 7 net + rules; 8 htop/nvtop interactive parity + pins CSV/multi-card + exec; 9 packaging + theme import.

**Why.** The blocker: arc 1 was ~15k lines, twice astral-watch, and built source data unrendered for six arcs. The reviewer's split is adopted with one partial rejection — retrowave and the htop cores tier stay in arc 1 because the meta-requirement is a visually impressive, real first arc and the trimmed loader is needed for gradient bars anyway; GPU, journal, table, inherits/overrides/WCAG, flourishes/effects and most CLI move out. Pins still follow immediately (signature tile), edit mode lands by arc 4 as a named product goal, audio precedes Winamp because Winamp consumes audio bands.

## D23 — Extensibility

**Decision.** Static registration via Cargo features now; gridwatch-app is a library so third parties assemble their own binary; an exec (JSON-lines) component in arc 8; cdylib/abi_stable rejected; WASM deferred to an optional post-1.0 host crate.

**Why.** ABI-stable plugins need repr(C) mirrors of ratatui types and a locked toolchain for no isolation; WASM costs ~50 MB of deps and a second rendering abstraction; the serde-snapshot + draw-only contract keeps both doors open.

## D24 — Process tables in the large tiers; tool-parity tiers are zoom-only

**Decision.** The htop `table` tier and a new gpu `procs` tier show a top-N process table (default `table_rows = 10`, never fewer than 5) with the tools' own column sets — htop's Main screen `PID USER PRI NI VIRT RES SHR S CPU% MEM% TIME+ Command`, nvtop's `PID USER DEV TYPE GPU% ENC DEC GPU MEM CPU% HOST MEM Command` — sharing one `/proc` scan and joining GPU rows by PID (§8.1). Both land in arc 2. The `full` tiers (htop screens/F-key bar, nvtop's sortable table with the signal menu) become `zoom_only` so a 6x3 grid tile shows the dashboard face and `z` gives the whole tool.

Supersedes D16's "process lists and the full tier follow in the parity arc"; amends D07 (tiers gain `zoom_only`) and D22 (arc-2/arc-8 contents). The column strings above are as first written; D26 corrects them to htop's printed widths and nvtop's default set (`GPU`/`CPU` headers, ENC/DEC optional).

**Why.** Matt asked for PID lists with the htop/nvtop columns inside the larger widgets (2026-08-30). Under the previous "richest tier that fits" rule a 122×31 tile would have jumped straight to `full`, putting an F-key bar and screen tabs on the Overview page; and the GPU process list was scheduled for arc 8 while the gpu component shipped in arc 2. NVML per-process accounting is cheap (0.35 ms lists + 1.7 ms utilisation per second) and only runs while such a tile is visible.

## D25 — Performance requirements are acceptance gates, and the terminal and GPU driver are in scope

**Decision.** `docs/PERFORMANCE.md` states 21 ceilings (P1–P21) for the gridwatch process, the terminal it drives, the NVML driver it polls and the buses it shares, plus a measurement protocol (`scripts/perf/measure.sh`) and a per-arc gate table. A red cell blocks the arc's commit like a failing test. Two mechanisms are added to the design to meet them: timer phase alignment in the source supervisor (P5) and an unfocused throttle driven by crossterm focus events (P4, P21, `[perf] unfocused_fps = 2`).

**Why.** Matt asked that the TUI not be a CPU or GPU hog (2026-08-30). Ptyxis is a GPU client (`nvidia-smi` lists it as `C+G`, 44 MiB), so bytes written to the terminal are GPU work by proxy beside the game; NVML calls execute in the driver; a capture stream can change PipeWire's quantum for everyone. Budgets that name the consumer and the tool that measures it are the only kind that survive nine arcs.

## D26 — Verified corrections to the process tables and the performance ceilings

**Decision.** After a four-lens adversarial verification (htop 3.4.1 and nvtop 3.2.0 sources plus the installed binaries; measurability on torch; internal consistency) the following changed: (1) htop table widths are htop's printed cells (`USER` 10 + 1, `CPU%` auto-width, `PID` from `pid_max`), `Row_printKBytes` has three regimes, state colours follow `processStateChar`, and the narrow-width drop order is gridwatch's (htop scrolls) with a `command_min` trigger and the order `VIRT, SHR, PRI, NI, TIME+, USER, RES, S, PID, MEM%, CPU%`; (2) `hide_userland_threads` defaults to **true** (htop: false) so the grid scan stays pid-level; the `task/` walk exists only at `Detail::Columns`; (3) nvtop's default column set omits ENC/DEC, prints `GPU`/`CPU` headers, sizes `USER` to the longest name and labels merged rows `Both G+C`; the gpu `procs` tier's minimum drops to 80×18 so a 4x2 shows a 7-row table, and nothing falls back to `nvmlSystemGetProcessName` (it reads `/proc` itself); (4) `Component::demand(tier) -> Detail` carries detail from tiers to sources, and the gpu manifest lists `cpu` as an optional source so the zoomed gpu table is never blank; (5) tier minimums are derived from a rows-occupied table (`cores` 56×12, `table` 70×18; `header` 56×8, `charts` 56×12, `procs` 80×18) so the 120×40 dense hero tiles keep `cores`/`charts`; (6) `SourceCtx::sleep` became the zero-poll `sleep_until`, the gpu fast tier is 500 ms visible, fans are polled at 5 s, the power trace only when drawn, the process scan runs at 3 s on the grid, and a render cache blits unchanged instances; (7) ceilings renegotiated to what the measured costs support: P2 ≤ 6 %, P3 ≤ 10 %, P6 ≤ 25 KB/s, P8 restated as ≤ 1 + Σ cadences, P11's sum listed, P15 ≤ 20 ms / ≤ 1 % amortised, P19 gains mean-cost rows; P7b/P10b (6x3 visualizer at 60 fps) are measured in arc 5 and gated afterwards; (8) measurement methods that work unprivileged on torch: task-summed voluntary switches (the leader-only counter under-counts ~80×), Δ`wchar` from `/proc/<pid>/io`, `nvidia-smi pmon` in its integer units, the full `nvidia-smi` table for P12.

**Why.** The first draft stated widths "as htop prints them" that were not, promised the Main screen's look without saying whether threads are rows, set NVML and `/proc` ceilings the design's own measured costs already exceeded (slow tier 4.4 ms + procs 2 ms > 6 ms; five-file scan ≈ 20–29 ms, not 15), and named `perf`/`strace` although `perf_event_paranoid = 4` and `ptrace_scope = 1` block them. Every item above was reproduced on torch or read in the tools' sources before being applied.

## D27 — Reduced default column sets on the grid

**Decision.** On grid tiles the htop table defaults to `PID RES SHR S CPU% MEM% TIME+ Command` (no `USER`, `VIRT`, `PRI`, `NI`) and the gpu table to `PID DEV TYPE GPU GPU MEM CPU HOST MEM Command` (no `USER`; `DEV` auto-hidden with one GPU; `ENC`/`DEC` off as in nvtop). Every dropped column stays available through the `columns` option, and the zoomed `full` tiers default to the tools' own full sets for parity. Because the reduced sets fit in 56 columns, both table tiers' minimum drops to 56×18 — the same width as `cores`/`charts` — so the 120×40 dense 6x3 (59×18) now shows 5-row tables instead of falling back. Drop orders: htop `SHR, TIME+, RES, PID, S, MEM%, CPU%`; gpu `ENC, DEC, HOST MEM, TYPE, USER, CPU`.

**Why.** Matt (2026-08-30): "we don't need user, and virt. PRI and NI could be eliminated to save space and declutter … On GPU, we can remove user." On a single-user workstation USER is almost always the same name, VIRT is rarely actionable at a glance, and PRI/NI matter only when you are about to renice — which is a `full`-tier action anyway. Supersedes the default sets in D24/D26; the width and formatting facts in D26 are unchanged.

## D28 — `matrix`: the first showcase-class theme, with its own ceilings and a veil

**Decision.** Themes carry a `class`: `quiet` (default; every P-ceiling applies) or `showcase`. A showcase theme may run an **ambient layer** — a post-render pass over the whole frame at its own fps — and while it is active *and the terminal is focused* the S-ceilings in `PERFORMANCE.md` (S1 ≤ 15 % of one core, S2 ≤ 3 MB/s, S3 Ptyxis ≤ +15 % CPU / ≤ 5 % SM, S4 p95 ≤ 16 ms) replace P2/P6–P10; on `FocusLost` or pause the ambience freezes and P4 applies unchanged. `matrix` is the first: seeded katakana rain (half-width, one cell wide, Noto Sans Mono CJK JP fallback verified on torch; `ascii` fallback set) over a phosphor-green palette, a `veil` (default 0.6) that dims and partially covers tile content, a `decode` effect that resolves text on focus/reveal, and a governor that steps fps → density → gutters-only when the terminal cannot keep up. Hard readability rules that no theme can override: the focused tile, any tile with an active Warn/Crit alert, the banner/toasts and the key bar are never veiled; `V` peeks; `reveal_ms` after any key. Ships in arc 4 with the effects layer.

**Why.** Matt (2026-08-30): "add an extra theme for the matrix, where we can spend more GPU and CPU resources for matrix style animations. In that theme, we don't necessarily need to have all information visible and readable at all times." The performance requirements are gates, not a religion — the honest way to honour both is a class the theme opts into, ceilings that name what it may spend, and a rule that it spends nothing when you are not looking. The readability floor keeps the one thing the dashboard exists for (the pins alarm) visible through the rain.

## D29 — In `matrix`, the rain is the renderer: breathing, deposit, afterglow

**Decision.** The static veil of D28 becomes a phase model. Each tile breathes `rain → resolve → hold → dissolve` on a scheduler (`period_s = 24`, `readable = 0.45`, `pattern = "wave"` by grid column, `min_readable = 2`, early `resolve` when the tile's data changes after `min_rain_s = 4`). In `resolve` the droplets **deposit** the tile's real cells from the render cache as they pass (forced completion at phase end); every deposited cell carries an **afterglow** — its colour is the `Rain` gradient sampled by time since it was written, head-white to the text role over `persist_ms = 2200` — so brightness encodes recency; in `hold` only cells that changed since the previous generation are re-deposited, so a live value flares and cools while the rest of the tile rests: the refresh itself is the effect. `dissolve` returns a tile to rain. Focused and alerting tiles, the banner/toasts and the key bar are pinned in `hold`; `V` resolves the page, `B` pins everything. The governor gains a "lengthen the breath" step; the state is ≈ 70 KB per frame of cells. Ships in arc 4b with the rest of `matrix`.

**Why.** Matt (2026-08-30): "matrix mode needs to occasionally have the components render … some dynamic breathing pattern of phasing in and out of the readability phase and the character rain mode … how the rain transforms into something that lasts and how the brightness lasts and how those effects could be used to refresh the widgets rendering." A veil that merely dims is decoration; a rain that writes the widget, remembers when it wrote each cell, and only rewrites what changed is both the look he described and an honest visualisation of the data's cadence. Supersedes the "dims and may overwrite" wording of D28; the S-ceilings and readability floor of D28 stand, and S2 is averaged over a breath cycle.

## D30 — Materials: the rain renders the module

**Decision.** Components render into a `Surface` — the ratatui `Buffer` plus an `IntentBuffer` (3 bytes per cell: kind, level, freshness) that the ui widget helpers fill as they draw (`BarFill`, `BarEmpty`, `Spark`, `Gauge`, `Spectrum`, `Number`, `Text`, `Chrome`, `Alert`, `Empty`). A theme maps each intent kind to a *material*: static ones (`blocks`, `shade`, `braille`, `ascii`) are applied at render time; the animated `rain` material (showcase-class only) is applied by the ambient layer every ambient frame from the cached intents, so bars, sparklines, gauges, the spectrum and big digits are *made of* rain under `matrix` and stay continuously legible as rain, while only text goes through the D29 breath cycle (plus idle re-rain at `text_flicker`). `Alert` is never re-materialised. `assert_intents_complete` fails a component that paints unmarked cells; a material snapshot test pins each material's look. The `IntentBuffer` plumbing ships in arc 1 (before the widgets multiply); the `rain` material in arc 4b.

**Why.** Matt (2026-08-30): "could the rain blend with the module rendering, like could the rain render the module?" Overlaying rain and depositing finished cells (D28/D29) treats the widget as an image to reveal; letting widgets state what each cell *is* and letting the theme choose the material makes the rain a renderer rather than a curtain, resolves the readability tension for every quantitative widget (a bar of glyphs is still a bar), and follows the rule the whole theme system rests on — components never name a glyph or a colour. Three bytes per cell and one extra argument to `render` is the entire cost of keeping that door open from day one.

## D31 — The rain lights the module (supersedes D29 and D30)

**Decision.** Under `matrix` the finished tile from the render cache is the target image and the falling rain is the brush. Each content cell carries `lit ∈ [0, 1]`: when a droplet head passes it, that frame shows the rain glyph at head-white and the next frame shows the module's own character fully lit; `lit` then decays over `fade_s` (12 s) through the cell's own colour to `floor` (`TextGhost`), so the module slowly fades until the next characters fall. Steady sparse rain (`density` 0.20) keeps cells shimmering; a dense **sweep** every `sweep_s` (20 s) re-renders the whole page in one coherent fall, which then fades together. `relight_on_update` re-lights a cell the moment its value changes, so live values stay bright and static ones fade. Nothing is phased, mutated or re-materialised: bars stay bars, text stays text. The focused tile, alerting tiles, banner/toasts and key bar are always fully lit; `V` re-lights the page, `L` locks everything lit. The D29 phase machine (rain/resolve/hold/dissolve, breath scheduler, deposit, delta glow) and the D30 intent/material model (`Surface`, `IntentBuffer`, rain-as-bars) are withdrawn; `render(&self, cx, buf)` keeps its original signature and arc 1 loses the intent plumbing.

**Why.** Matt (2026-08-30): "that render is bad. I was asking if the vertical rain could be used to render the module. Say those characters stay lit and slowly fade such that the module slowly fades until the next cycle of characters fall." D30 replaced the module's glyphs with rain glyphs, which destroyed the widgets; D29 chopped the effect into phases and curtains. The request is simpler and better: keep the module exactly as rendered, and let the rain be the only thing that decides how visible each of its cells is — brightness as memory of the last fall. It is also cheaper (no per-cell intents, no phase state) and keeps every earlier guarantee (readability pins, freeze on focus loss, governor, determinism).

## D32 — Components describe, themes render: the view tree, and the wire protocol as the public plugin API

**Decision.** `Component::render(&self, cx, buf)` becomes `Component::view(&self, cx) -> View`: a small semantic tree (`Text`, `KeyValue`, `Gauge`, `Bars`, `Sparkline`, `Chart`, `Table`, `BigNumber`, `Stack`, and a `Custom` escape hatch that still styles only through theme roles). The shell orchestrates `tier → view → theme.renderer() → cache → ambient`; the default `Renderer` lives in `gridwatch-ui` and is parameterised by a per-theme `[widgets]` table (`gauge = "bar" | "line" | "block"`, `bars`, `sparkline`, `table_header`, `big_number`), so themes own *form* as well as paint. The render cache also keys on the view hash. Modules are plugins against the two contracts `Source` and `Component` with a `Manifest` and a `Registry`; in-process plugins stay static (no stable Rust ABI), and the **public** plugin API is the `exec` wire protocol (arc 8): JSON lines — manifest, samples, render requests answered with a view tree, keys, commands, status — validated by JSON Schemas under `schema/`, with a `contract` number; WASM later on the same schema. Every boundary format (config, layout, theme, journal, view, manifest, exec) gets a schema and CI-validated fixtures.

**Why.** Matt (2026-08-30): "should these modules be plugins following any design patterns and APIs? should some orchestrator render the theme on what the module provides?" The modules were already plugin-shaped (Manifest/Registry, Strategy tiers, Command side effects, journal replay), but a component that writes cells gives the theme authority over paint only, and an external plugin would have to emit cells. A view tree is the right altitude — D30 failed because it let the theme substitute glyphs inside a finished widget; D32 lets it substitute the widget. It also makes out-of-process plugins safe and trivial, gives snapshot tests a semantic layer, and costs one enum plus the widget helpers already planned. D31 (rain lights the module) is unaffected: it operates on the renderer's output.

## D33 — Spec-driven at the seams

**Decision.** The project follows spec-driven development where it pays: at the seams. The spec is `docs/ARCHITECTURE.md` (contracts with real signatures), `docs/KEYS.md` (the generated metric catalogue), `schema/*.json` (every file and wire format), `docs/PARITY.md` (per-tool feature checklists) and `docs/PERFORMANCE.md` (measured gates). An arc begins by updating the spec sections it implements and writing its acceptance criteria and gates in `ROADMAP.md` — before code; the adversarial review checks the implementation *against the spec* and flags drift in either direction; a spec change is a `DECISIONS.md` entry. Specs are executable wherever possible: schemas validate fixtures in CI, `gridwatch keys` regenerates `KEYS.md` and fails on drift, view-tree and renderer snapshots pin behaviour, `PARITY.md` rows are ticked by tests or by hand with a note. Internals are deliberately *not* spec'd: signatures in `ARCHITECTURE.md` are binding for seams (Component, Source, Store API, theme/layout/config files, the wire protocol, the journal) and advisory for private code.

**Why.** Matt (2026-08-30): "are we going to follow a spec driven development? would that be valuable at ensuring the quality and extensibility of this project?" Yes, for a multi-session solo project whose value is its contracts: the spec is what a future session (or a plugin author) reads instead of the code, executable specs turn review into verification, and keeping internals free avoids the paperwork that kills solo momentum. The plan was already most of the way there; D33 names the practice and adds the three missing pieces — schemas, the spec-first ritual per arc, and drift checks.

## D34 — Figure-ground: only the rain draws (amends D31)

**Decision.** Under `matrix` the ambient layer is the sole compositor. The finished frame from the render cache is a mold, never composited to the screen; every visible cell was put there by a droplet. A head crossing empty space prints a katakana that fades at `trail_ms` (900); a head crossing content shows the katakana for one frame, then the module's own character, which fades at `fade_s` (12 s); `floor` defaults to **black** (was `TextGhost`), so modules fade out entirely between falls. Chrome — borders, titles — is content and is printed and fades like everything else. The pinned exceptions (focused tile, alerting tiles, banner, key bar) are continuously printed at full brightness. Everything else in D31 stands: sweeps, re-light on update, `V`/`L`, the governor, freeze on focus loss, determinism, ~70 KB of state.

**Why.** Matt (2026-08-31): "can we have the matrix rain render the widgets, rather than the widgets overlaying on top of the rain?" D31 still described two layers — content at some brightness with rain around it. One field is the honest version of the idea: there is only rain, and a widget is the rain's memory of having fallen through its shape.

## D35 — Open decisions resolved; the project is named gridwatch

**Decision.** Matt resolved the open list on 2026-08-31: (1) name **`gridwatch`** — repo `github.com/mbeaman/gridwatch`, crate/binary `gridwatch`, workspace crates `gridwatch-*`, env `GRIDWATCH_*`, config `~/.config/gridwatch/`; every living doc renamed, historical research/design-review digests left as written; the on-disk workspace directory stays `~/workspace/opsTui` until the repo exists. (2) Arc 1 showcase cut (retrowave + cores tier stay). (3) 12 × 6 grid. (4) Two config files. (5) Mouse capture on. (6) Audio 30 fps / latency 1024 with `fps` and `low_latency` as explicit config knobs. (7) Process actions in arc 8 behind confirm + `readonly`. (8) ICMP probes on, public-IP/rDNS off. (9) astral-watch v0.8.0 + three PRs before arc 3, in an astral-watch session. (10) Optional RAPL udev rule shipped. (11) Winamp classic skin. (12) Matrix light defaults deferred to arc 4b tuning — "matrix mode still needs a lot of improvement to balance rendering the theme and usability of the modules, but we can iron that out later."

**Why.** Eleven of twelve confirmed the recommended defaults; the name is the one identity choice only Matt could make, and `gridwatch` describes the product (a grid of watchers) while staying free on crates.io and GitHub (verified 2026-08-30). Nothing blocks arc 1a.

## D36 — Fable builds the foundation, Opus builds the verticals

**Decision.** Model division of labor, recorded in `docs/MODELS.md`: Fable 5 owns the seams and foundation (arc 1a's core implementation, `schema/`, the testkit, `measure.sh`), writes a decision-complete brief per arc (`docs/briefs/`), judges the review gates (`docs/REVIEW.md` templates), pre-shapes or post-reviews the gnarly kernels, and makes every seam change; Opus 5 implements the verticals against a brief — one brief per session — plus fixtures and mechanical breadth, escalating seam questions instead of improvising. The gates (commit-before-review, read-only guard, reproduce-before-fix, spec verification, performance rows) apply identically to both. Briefs for 1a and 1b are written and adversarially verified; later briefs are written at arc start per the D33 spec-first ritual so they cannot drift from what earlier arcs taught.

**Why.** Matt (2026-08-31): "are there any tasks that fable should execute to establish a foundation for opus to leverage while working on implementation verticals?" The plan already concentrates risk in the seams and pins behaviour with executable specs — exactly the split that lets a fast model implement safely. What was missing was the packaging of Fable's context into artifacts another model can execute from: the briefs, the gate templates, and the escalation rules. Writing all nine briefs now was rejected: early arcs will teach things later briefs must absorb.

## D37 — Brief-verification fixes: View::Segmented, self-contained themes, and the corrected gates

**Decision.** The adversarial pass over `MODELS.md`, `REVIEW.md` and the arc 1a/1b briefs (3 lenses, 27 findings — 1 blocker, 10 major) produced one seam change and a set of corrections, all applied. Seam: **`View::Segmented { label, segments: Vec<(Role, f32)>, text }`** joins the view tree — htop's four-segment CPU meter and its mem/swap bars were not expressible in the ten-node tree while `Custom` was banned for that component, and segmented meters are a general primitive. Spec clarifications: a theme file is **self-contained** until `inherits` lands (`theme.schema.json` requires every role and all eight gradients); `sysinfo` joins the deny bans everywhere the list appears; the arc-1 shipped layout swaps the `amp` slot for a `sources` tile until arc 6; pure edit ops + proptests are explicitly arc-1 scope. Brief corrections: the layout-tier assertions now name shipped placements only (a configured 4x2 yields `cores`, not `meters`); retrowave ships complete with the four gradients the §9 excerpt omits; the F12 HUD has one owner (1a); `view = "table"` warns-and-falls-back in 1b per §4.6; the CPU bar's virt segment is steal + guest; 1b gains the formula-fixture task (`fixtures/procfs/`) and the closeout-docs task (PARITY htop section, THEMES glyph check, README dump + PNG, CHANGELOG). Gate fixes: `REVIEW.md`'s snapshot recipe now actually captures the stash commit (`snap=$(git stash create); git tag review-snap-<date> "${snap:-HEAD}"` — the old wording tagged HEAD and protected nothing), adds the one-time `cargo install cargo-deny cargo-audit cargo-insta`, and replaces interactive `cargo insta review` with per-snapshot inspection; `MODELS.md` now assigns arcs 4a/4b/8 and adds REVIEW/PERFORMANCE to the arc-end reading set.

**Why.** The briefs exist so an Opus session can implement without improvising; the verification found exactly the failure modes that defeat that purpose — an unimplementable centerpiece task, a test assertion that contradicts the solver, an excerpt mistaken for a file, dead-scope double assignment, unowned deliverables, and a safety ritual that didn't do what it claimed. Every fix was validated against the quoted spec text before applying (the patch asserts the cited wording verbatim).

## D38 — The backlog file

**Decision.** `docs/BACKLOG.md` is the single home for everything known but not scheduled: the pre-flight steps before arc 1a (repo creation, the directory-name/memory-key wrinkle, CHANGELOG scaffold), unscheduled wants (a `disk` component, a combined `power` tile, opt-in out-of-band Crit notification, journal time-travel, state persistence, multi-GPU rendering, plugin verify tooling, container awareness, the crates.io chain through astral-watch), hardening items (focus events under tmux/ssh, exec-plugin security posture), and recorded won't-dos (delay-accounting columns, cdylib plugins, Sixel art, pre-1.0 WASM). Pulling an item into an arc requires a DECISIONS entry; the file migrates to GitHub issues once the repo exists, with this file as the index. Seeded from the PE sweep plus a three-lens gap hunt over the corpus (product gaps / operational risks / doc debt).

**Why.** Matt (2026-08-31): "anything remaining before we proceed to the next task? are there any tasks we want to include in the backlog?" Nine arcs of roadmap plus a growing pile of "later" scattered across six documents is how deferred work gets lost or silently re-litigated; one bucket with dispositions is how a solo multi-session project keeps its promises cheap.

## D39 — Gap-hunt closure: the last unowned items find owners

**Decision.** The final three-lens gap hunt (20 findings) is applied. Spec/doc fixes now: status banners reflect Matt's 2026-08-31 review; the WORKSPACE tree includes the planning corpus the first commit contains; `config.toml` carries `schema = 1` from arc 1a; the start-focused default (absent focus reporting ⇒ assume focused) is spec'd in §11 and the 1a brief; the measure protocol states that Matt starts the "beside a game" neighbour; the local MSRV gate installs the 1.88 toolchain or is declared CI-only; the Makefile and `.cargo/config.toml.example` join the 1a scaffold; the measured-terminal-size propagation is owned by 1a's Done-when; the 120×40 layout fixture and `torch-audio.jsonl` have owning arcs; arc 9's doc list gains ADDING-A-COMPONENT and CONTRIBUTING. Moved into arcs: PARITY's htop section marks the header-meter rows out (1b); the alerting stance becomes a DECISIONS entry when the overlay ships (arc 3: gridwatch is a viewer, the astral-watch service alerts); the astral-watch crates.io publish decision joins the pre-arc-3 session (a git dep can never be published); the exec security posture moves into arc 8's deliverables. New backlog: machine-readable output (`--once`/`--json`/exporter — prior-art pattern #10, dropped silently until now), `gridwatch-netd`, htop meter breadth. New won't-do: Winamp realism limits beyond MPRIS. The backlog's false "diskstats already read" claim is corrected.

**Why.** Matt (2026-08-31): "anything remaining before we proceed? tasks for the backlog?" The hunt's value was precisely the items no single doc owned — a parity commitment that would have failed silently, a publish chain that quietly expired, and a safety-alert stance everyone assumed someone else had written down.

## D40 — Session topology: match the driver to the work

**Decision.** Recorded in `docs/MODELS.md`: Fable-as-orchestrator-with-Opus-subagents is *not* the universal pattern. Vertical arcs run as **Opus main-loop sessions** (sequential, stateful builds want one agent with full context; Fable's oversight is asynchronous via brief + gates), with one hybrid mandated: the arc's adversarial **verify/refute stage runs on Fable subagents inside the Opus session** (`model: fable`), so the strongest model judges findings without sitting in the driver's seat. Fable main-loop sessions own seams/briefs/judging and may fan out cheap subagents for breadth. Fable-orchestrator + Opus-worker topology is reserved for wide, independent, spec-complete task sets (arc 8 parity breadth, arc 9 packaging matrix, fixture farms). Rule of thumb: sequential + stateful → one strong main loop; parallel + independent → orchestrate.

**Why.** Matt (2026-08-31): "should every session run fable as an orchestrator and opus as a sub agent executing tasks?" Orchestration serializes every task through prompts — pure overhead when tasks depend on each other, and it burns the expensive model's context supervising work the brief already constrains. The places Fable's depth actually changes outcomes are the brief, the seams, and the refutation of findings — all reachable without owning the session.


## D41 — deterministic wall-clock seam (arc 1a, 2026-08-31)
`RenderCx` gains `pub tz_offset_s: i32`: the app computes the local-UTC offset once at startup (libc `localtime_r` in `app::sys`, the crate's single `#[allow(unsafe_code)]` module) and components derive local wall time from `cx.wall + tz_offset_s` — no `chrono`, no per-frame syscalls, testkit passes 0. Under `Clock::Virtual` (shot, replay, tests) `wall` comes from the virtual clock instead of `SystemTime::now`, making `gridwatch shot --seed N` byte-deterministic end to end. Logged per D33 as a seam addition to §4.6.

## D42 — arc-1a review outcomes at the seams (2026-08-31)
The Template A review (4 lenses, 15 agents, 11/11 findings adversarially confirmed) forced these seam rulings:
- **§5 render cache implemented in full**: the key is `(source generations, tier, inner rect, theme id, focused, zoomed)` **plus the view fingerprint** (hash of the snapshot serialisation, `ui::view::fingerprint`); the spec's `animation frame` term joins the key when `Animated` lands in arc 5. This is the backstop that keeps store-reading tiles with empty manifests (the sources tile) honest.
- **`generation` counts data batches only.** A `ControlMsg::Status` does not bump it; repaint-on-state-change is carried by the view fingerprint plus the control-drain dirty flag. Components must not treat `generation` as "anything changed".
- **`Command::Run(ActionId, Box<dyn Action>)`** stays as implemented (spec said `Run(Box<dyn Action>)`): the id makes "keys in, commands out" tests addressable. `Action::run` takes no `ExecCx` until the executor thread exists (arc 8); §4.6 amended.
- **§6 dense mode**: the tab bar is now hidden (as spec'd); `Block::merge_borders(MergeStrategy::Exact)` for shared-border junctions is **deferred to arc 4** (visual polish) — until then dense borders overlap without junction merging. BACKLOG entry added.
- **Supervisor control channel**: the handle's sender is re-pointed at every restart (`Arc<Mutex<Sender>>`); a control sent in the instant between generations is dropped — acceptable for telemetry tuning, Stop rides the atomic flag. `SourceCtx` now carries the supervisor-owned restart count and stamps it onto every status.
- **D39 status**: the 250×70 Ptyxis assumption was exercised headlessly everywhere (smoke, perf rows, tests at 131×37/120×40/80×24) and held; the real terminal size still needs one `stty size` from Matt, then propagate or mark "assumption held".

## D43 — three catalogue changes the htop tier needed (arc 1b, 2026-08-31)

**Decision.** Arc 1b adds two keys to `keys/cpu.rs` and corrects the semantics of a third; `ARCHITECTURE.md` §8's htop key list is amended in the same commit (D33: the spec moves with the code, and drift is checked in both directions).

1. **`cpu.topology` (Record `CpuTopology { die_of, core_of, die_temp }`), additive.** The `cores` tier draws two CCD blocks of SMT pairs, and on torch CCD0 is `{0–7, 16–23}` — *not* derivable from CPU numbering, and htop's own `coresPerCCD` heuristic needs `core_id` to reach the same answer. A component may not read a device (§4.6), so the source must publish the map it already reads from sysfs `topology/{die_id,core_id}`. Published once per source generation, latest-only, with a journal decoder; `demo::CpuSynth` publishes torch's map on the same schedule so demo and live cannot drift (§12.5).
2. **`swap.cached_b` (scalar), additive.** htop's SWP meter draws `used` and `cache`; without `SwapCached` the second segment could not exist.
3. **`swap.used_b` semantics corrected, not additive.** Its doc said `SwapTotal − SwapFree`; htop's `usedSwap` is `SwapTotal − SwapFree − SwapCached` (`LinuxMachine.c`, 3.4.1). The key now carries htop's number, which is a **changed value**, not a new one — on torch swap is unused so nothing visible moves, but a reviewer diffing behaviour should see it called out rather than discover it.

3b. **`mem.used_b` semantics corrected, not additive** (added after the arc-1b integration review, which caught the omission). Its doc said `used = total − available`; htop's is `MemTotal − (MemFree + Cached + SReclaimable + Buffers)`, with htop's own `MemTotal − MemFree` fallback when the parts overshoot. The key now carries htop's number — the same category as the `swap.used_b` correction above and the more visible of the two, since it is the figure the MEM meter's bar and its `used/total` text are built from.

Also recorded here rather than left implicit: the aggregate CPU meter reads the **unlabelled** `cpu.breakdown` (`/proc/stat`'s `cpu` line), with `.idx(n)` staying per core; and **`tasks.kernel` is deliberately unproduced in 1b** — counting kernel threads means `PF_KTHREAD` from every `/proc/<pid>/stat`, which is the pid-level scan `Detail::Table` gates in arc 2. The synth drops it too, so no demo-driven snapshot claims a number the live tile cannot show (PARITY.md carries the row).

**Why.** §15 step 1 makes "new data ⇒ new typed key" the documented path for an implementation vertical, and D41 set the precedent for logging such an addition instead of escalating it: nothing in the `Key`/`KeyMeta`/`RecordValue` machinery changes, only the vocabulary grows. They are logged together because they arrived together, but they are different categories — two additions and **two** behavioural corrections (`swap.used_b`, `mem.used_b`) — and only the corrections can change a number on screen.

## D44 — the stats log samples on its own clock (arc 1b, 2026-08-31)

**Decision.** `--stats-log` writes its JSON line on a 1 Hz wall clock instead of on the heartbeat redraw. No counter, no HUD field and no instrumentation semantics change — only *when* the line is emitted.

**Why.** The heartbeat fires only when nothing else drew for a second, so the moment the app became busy — exactly the state whose P6/P8/P19 rows the gate needs — the log went silent. Measuring arc 1b produced three lines in 44 seconds while the app was drawing two frames a second; the numbers in `PERFORMANCE.md` could not have been taken without this. The brief's "do not rewrite the instrumentation" (task 4) is respected: the HUD, the counters and the byte accounting are untouched.

**Amended after the arc-1b review.** The review confirmed that P6 and P18 were being ticked without the evidence their own rows name, so the stats line grew the three fields that carry it and nothing else changed: `bytes` (the terminal writer's own counter, so P6's "the HUD counter must agree with Δ`wchar` within 5 %" is checkable from one run — measured **1.99 %**), and `first_frame_ms` / `sources_live_ms` (P18's two timestamps, also added to the F12 HUD so `--stats` shows what P18's row promises). Measuring them immediately found two real defects, both fixed in this arc: the frame loop parked on input for up to 250 ms *before* its first draw (first frame 251 ms → **1 ms**), and both cpu sources waited for their first cadence boundary before sampling, so with the demand still `Hidden` the first batch landed at 3.0 s, over P18's 2 s clause (→ **252 ms**, and the first delta now arrives a period earlier).

## D45 — tiers are cumulative in *information*, not in layout (arc 1b, 2026-08-31)

**Decision.** §4.6's "tiers are cumulative supersets; tier *i* draws tier *i−1* plus `adds`" is binding on the *information* a tier shows, not on the shape it shows it in. A richer tier may express a poorer tier's datum differently — the htop tile's total CPU% is big digits at `big-number` and the CPU meter's bar text at `meters` and `cores`, and the big digits do not reappear. Where a richer tier genuinely has room for a poorer tier's element it must draw it: the sparkline (`tiny`'s own content) therefore outranks the pressure row when only one spare line remains, which is the one behavioural change this ruling forced. A `Tier`'s `adds` list must name what that tier actually draws, so the ladder stays auditable from the manifest.

**Why.** The arc-1b review found the literal reading violated: `meters` drops the big digits, and at exactly 30×6 it was dropping the sparkline too. The literal reading cannot survive contact with a rect — 3 meter rows plus 4 rows of big digits plus a task line does not fit the 30×6 minimum the same spec sets — and htop itself re-flows the same numbers per meter mode. Naming the interpretation is cheaper than either weakening the tier ladder or pretending the layout obeys a rule it cannot. The sparkline half *was* a real defect and is fixed; the big-digit half is now spec, not drift.

## D46 — the harness gains a user-facing axis (2026-09-01)

**Decision.** `docs/TESTING.md` is adopted as the testing contract. Five layers become CI gates: content oracles in the component sweeps (non-blank, per-tier signatures, no fabricated data), an app-level size lattice plus a resize sequence, the binary run under a real pty (no tty, one row, first frame, resize, clean quit), an error-visibility rule in §11 (before the alternate screen → inherited stderr; after → log *and* UI), and a mandatory `user-path` review lens with the pty test's transcript in every arc report. `assert_never_panics` becomes `assert_renders_everywhere`; components gain `signature(tier)`.

**Why.** Arc 1b passed every gate and two adversarial reviews, then failed twice in Matt's first thirty seconds of use: a non-tty exited silently and a too-small terminal drew nothing. Both lived in code no test executed (`run_terminal`) or in a branch every test accepted (a blank buffer). The harness was built for determinism — headless, seeded, byte-exact — and that axis cannot see what happens at the boundary with a real terminal and a real person. The retrospective is in `TESTING.md`; the rule it leaves behind is that "didn't panic" is never a passing assertion on its own.

## D47 — arc 2 brief: the seams are decided before the code (2026-09-01)

**Decision.** `docs/briefs/arc-2.md` is written and fixes six seams the implementer must not move: (1) the journal line format — JSON Lines with a header object then `{t, b|st|al|in}` lines, samples as `[name{label}, datum]` whose JSON *type* selects Scalar/Vector/Record, names interned through `lookup`, `--tables off` omitting `proc.table`/`gpu.procs`; (2) replay is a `Source` (`JournalSource`) that drives `Clock::Virtual` and re-emits through the normal channels, so nothing downstream can tell replay from live; (3) the recorder is a bounded-channel tee in the frame loop to a writer thread and may drop, never stall; (4) the gpu key catalogue, all `{dev}`-labelled, with `GpuInfo`/`GpuSpec`/`Throttle`/`GpuProcs` Record shapes, `0x2B85` = RTX 5090, effective load computed in the component; (5) per-source/per-component Cargo features arrive with the second source, as WORKSPACE.md planned; (6) the new tiers carry D46 `signature`s and the pty suite must draw both tables in `--demo` and reach a frame in `--replay`. Arc 2 runs as two Opus sessions, 2a (journal, pid scan, htop table, SVG/KEYS/COMPONENTS tooling, fixtures) then 2b (gpu source and component, nvtop parity), because 2a has no hardware dependency and produces the fixtures 2b's tests need. `ARCHITECTURE.md` §4.5 gains the concrete line format.

**Why.** D36: the brief exists so an Opus session never improvises at a seam, and arc 1b's review found exactly the drift that happens when a seam is left as prose ("cumulative tiers", the task-line wording). The journal format and the GPU vocabulary are the two seams every later arc reads back — replay fixtures, alert rules, the `exec` protocol — so they are the ones to freeze now, with the nvtop digest's verified NVML behaviour as the evidence.
