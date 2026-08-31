<!-- Adversarial critique (findings resolved in ARCHITECTURE.md §16 / DECISIONS.md). Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Critique — product

PRODUCT & PE REVIEW — plan vs. user goals (modular grid with per-footprint rendering; htop/nvtop/astral-watch parity; net/audio/Winamp; retrowave/modern themes; showcase quality; arcs with review-before-commit) and roadmap ordering

## Blocker (1)

### Arc 1 is roughly twice the size of all of astral-watch and cannot be reviewed as one arc

Arc 1 ships six crates, the full Store (incl. journal record/replay/JournalSource and a virtual clock), the complete theme loader (roles, $palette, inherits, Oklab gradient LUTs, glyph tiers, WCAG gate, ColorMode ladder, mono), four themes, seven widgets, the layout engine with dense/starved/too-small ladder, a procfs cpu source with a full process table, an NVML source with process lists + process utilisation + NotSupported pruning + nvidia-smi fallback + const spec table, four components, the app loop with dirty gating/pages/focus/capture/zoom/dense/theme cycle/pause/HUD/screenshot, five CLI subcommands, plus snapshot/proptest/replay-determinism infrastructure and a perf doc. That is ~15k lines of Rust; the entire astral-watch crate is 7.5k lines. It also builds source data nobody renders for six arcs (GPU process lists/utilisation and the spec table are only consumed by the gpu `full`/`charts` tiers in arc 7). Arc 2 (three pin backends + five tiers + AlertLog/overlay/alerts tile + capability probe/doctor + staleness + hot reload + two themes + upstream PRs + two fixtures) and arc 7 (htop full parity + nvtop full + pins modes + exec component) have the same problem; arc 3 pads with a theme importer. The plan's own §16 says all three proposals overscoped arc 1 and then keeps it.

**Fix:** Split arc 1: v0.1.0 = store core (no journal), ui core with the layout engine + `modern`/`mono` only, app loop + pages/focus/zoom + HUD, `clock` + htop `big-number`/`meters`, demo Synth, snapshot/no-panic testkit, `run --demo` + `shot`. v0.2.0 = gpu badge/gauges/header + `charts` (see the parity finding), htop `cores`/`table`, `retrowave`, record/replay + determinism test, remaining CLI. Move every source feature to the arc whose tier first renders it (GPU process lists/spec table/nvidia-smi fallback to the nvtop-full arc). Trim arc 2 to i2c + exporter (CSV tail to arc 7 where it is verified anyway) and drop `theme import` to arc 8.

## Major (7)

### Alert and status messages ride a lossy try_send channel shared with 60 Hz audio batches

`SourceCtx::emit/status/alert` all `try_send` into one bounded `sync_channel<Msg>(4096)` and 'drops are counted into status'. The pins source publishes Overload/Disconnected transitions as `Msg::Alert` on that same channel. Any render-thread stall (a long terminal write on VTE, a slow `apply`, a modal) that lets the audio publisher fill the queue silently drops the exact message the signature feature exists to deliver; the Recorder tee inherits the loss. 'Missing is a state, not an error' does not hold for a dropped alarm — it is invisible.

**Fix:** Give control-plane messages a non-lossy path: `SourceCtx::alert`/`status` use blocking `send` (sources are on their own threads, so blocking is fine), or add a second unbounded `control` channel that the frame loop drains before data batches. Assert in a unit test that an alert emitted while the data channel is full is still applied, and that the journal contains it.

### min_terminal = 100×30 and rows = 6 contradict the tier minimums; the degradation ladder never triggers where it matters

With `rows = 6`, gap 1 and 2-row chrome, a 100×30 terminal gives ~1–2 inner rows per grid row — every 1-row tile is a chip and even 2-row tiles are Tiny — yet stack mode only starts below 100×30, so the ladder's last rung is unreachable on exactly the terminals that need it. The plan's own 120×40 numbers show it: 2x1 inner = 17×3 fails htop `big-number`'s min 12×4, and pins `watts-badge`, gpu `badge` etc. will all be chips on a laptop. The auto-dense step is also underspecified: 'per frame: configured → dense → starved → stack' has no trigger; if it is 'any starved cell', a single small tile flips the whole page's border style and can oscillate on resize.

**Fix:** Derive the stack threshold from the layout instead of a constant: `min_rows = rows × (min_unit_inner_rows + chrome + gap) + shell_chrome` (≈ 6×7+2 = 44 rows for rows=6) and likewise for columns; make `rows = "auto"` the shipped default so laptops get 4–5 rows; trigger dense only from terminal size (a hysteresis band), never from a single starved cell; add the 120×40 snapshot matrix to arc 1 acceptance with an explicit assertion that no tile in the default layout renders as a chip.

### Tier composition, `view`, and the leftover SizeClass/Footprint concepts are hand-waved

Tiers are listed as alternatives chosen by 'first whose min fits', but the research footprints they came from are cumulative (6x3 htop = meters + CCD cores + top-N table). As written, htop at the Overview 6x3 (122×31) selects `table` (min 70×16) and the per-core CCD blocks disappear; gpu `charts` vs `header`, pins `trend` vs `bars` have the same ambiguity. `view = "table"` in a placement and `RenderCx.view: Option<&str>` are passed to the component while the shell independently picks `tier: usize` — undefined when the named view's min does not fit or when both are set. `SizeClass` (five buckets) and `Shape` appear in the contract but 'only steer titles', and `RenderCx.footprint` is passed to `render` despite principle 6 ('tiers, not footprints, decide content').

**Fix:** Specify each tier as a superset of the previous one in the manifest tables (e.g. htop `table` = meters + cores-compact + table) and let a tier declare which sub-panels it drops first. Define `view` as a *preferred tier name*: honoured when its min fits, otherwise fall back to the richest fitting tier and show a small `view↓` chip. Remove `footprint` from `RenderCx`; keep `SizeClass` only as the argument to `title()` (or delete it and pass `inner.width`).

### Cross-arc data dependencies are inverted or unexpressed (Tccd temps, Winamp's audio bands)

htop's `cores` tier (arc 1) shows Tccd temperatures via `sensor.temp_c{k10temp:Tccd1}`, but the sensors source that produces that key is arc 4. Winamp's `shade`/`main` tiers (arc 5) consume `audio.bands` and `audio.sink`, yet its manifest lists only `mpris` as a source, and Demand is computed from the sources declared by visible cells — so with only the Winamp tile visible the audio source is Hidden, pw-record is killed after 10 s, and the 19-band vis goes dark. There is no notion of an optional source in `Manifest` (one `sources` list), so this cannot be expressed today.

**Fix:** Add `Manifest.optional_sources` (contributes to Demand, absence degrades the tier rather than skipping `build`). For arc 1 let the cpu source read k10temp `Tccd*` itself (a 20-line label-keyed hwmon read) and re-home it into the sensors source in arc 4 with the key name unchanged. Make Winamp declare `audio` optional and render the vis area as a static skin when the key is absent.

### nvtop parity is scheduled backwards and 'parity' has no testable definition

nvtop's core UI is the 10-minute rolling chart with selectable metrics; the plan defers the gpu `charts` and `full` tiers to arc 7, so from v0.1 to v0.6 the GPU tile has no history chart at all, while `Store::resample` — built in arc 1 precisely for this — goes unvalidated by a real chart consumer for six arcs. htop 'same functional behaviour' likewise lands in v0.7 after edit mode, effects, audio, Winamp and net. Beyond ordering, neither tool has an in/out checklist: htop meter modes Bar/Text/Graph/LED (the LED mode the research flagged as the retrowave gift), F2 setup / per-screen column selection, `e l s w x Y # p m`, and multi-GPU are simply absent, and the `gpu.*` keys carry no device label so nvtop's multi-GPU header blocks would require a breaking catalogue change later.

**Fix:** Move gpu `charts` (resample-driven, nvtop ring-buffer semantics, `-r` reverse) into the arc that ships the gpu component. Add `docs/PARITY.md` per tool listing every htop/nvtop/astral-watch feature as in-scope (arc N) / out (reason), and make arc 7 acceptance a diff against it. Label GPU keys by device index from day one (`gpu.util_pct{0}`) even with one card.

### Config sprawl: the same knob has two homes and edit mode still rewrites behaviour

`[sources.audio]` (fft/floor/tilt) vs `[[components]] options = { preset = "winamp" }`; `[sources.net] show/hide` globs vs the `lan` instance's `interfaces = [...]`; `[sources.cpu] hide_kernel_threads` (htop's `K`, a view toggle) in the source section. Sources are singletons per kind, so two audio instances with different sinks or `Command::Source(audio, SetSink)` change the sink for everyone — never stated. Instances with behaviour options live in `layout.toml`, the one file edit mode rewrites, which contradicts the stated reason for the split ('write-back never touches your hand-edited behaviour file'). Four files plus a themes directory for a personal tool.

**Fix:** State the rule: sources are singletons configured only under `[sources.<id>]` in config.toml; instance `options` are view-only (filters, presets, sort) and validated by the component's `Options` type, with a CI check that no key name appears in both. Move `[[components]]` into config.toml (the picker only places existing instances or a `kind:` shorthand that edit mode writes as a placement), keep layout.toml as pages/placements only, and fold `rules.toml` into `[[rules]]`.

### Testkit and demo placement violate the declared dependency direction

`opstui-ui::testkit::demo_store()` and `snapshot_matrix!` (used from `opstui-components` tests) need seeded demo data, but `demo::Synth` lives in `opstui-sources`, and the plan says components do not depend on sources and ui depends only on store. Either the direction is broken or `demo_store()` must reimplement Synth by hand — which then drifts from `opstui --demo` and breaks the 'demo and snapshots share the same seam' promise.

**Fix:** Move `Synth` into `opstui-store::demo` (it is pure seeded data generation with no system deps) and let `SourceDef.demo` wrap it; or explicitly allow `opstui-sources` as a dev-dependency of `opstui-components` and document the exception. Make the checklist step 'extend Synth' point at the store crate.

## Minor (4)

### Styled snapshot matrix will explode into hundreds of large, churn-prone files

Styled cell dumps (fg/bg/modifiers per cell) × 7 components × ~6 real sizes × 3 themes × tiers means several hundred snapshots of 100–500 KB each; every palette tweak or WCAG autofix change churns all of them, and the adversarial-review step degrades to rubber-stamping `cargo insta accept`.

**Fix:** Snapshot styled dumps at one reference theme (`modern`) per component/size; cover themes with a compact per-theme 'role swatch' snapshot (one line per Role and gradient stop) plus a few targeted `cell().fg` assertions; run-length-encode style runs in `dump::cells`.

### Crossterm leaks into the 'headless' store and ui crates via Msg::Input and on_key

`Msg::Input(crossterm::event::Event)` puts crossterm into `opstui-store` ('no ratatui, no system deps, builds in ~1 s') and forces the journal to serialise crossterm events; `Component::on_key(KeyEvent)`/`on_mouse(MouseEvent)` put it into `opstui-ui`, which is claimed to be ratatui-core/-widgets only. Input also queues behind data batches on the same channel.

**Fix:** Define `opstui_store::InputEvent` (Key/Mouse/Resize/Paste mirror) converted once in the input thread, use it in the Component trait, and give input its own channel drained first each frame.

### Performance headline does not match the default layout

The default Overview page places `viz`, which reports `Animated{fps: 60}` whenever visible, so the shipped default state is the '<8 % with the visualizer at 60 fps' case, not the '<2 % beside a game' headline; the research recommended 30 fps default with 60 opt-in.

**Fix:** Make the visualizer cadence a config value defaulting to 30 (`[sources.audio] fps = 30`) with 60 opt-in, or keep `viz` off the default Overview page and on the Audio page; state the budget per page in PERFORMANCE.md.

### Arc-internal inconsistencies that will bite during arc 1

`clock` is `Chrome::Borderless` in arc 1 but 'Chrome honoured by the shell' is arc 3; arc 1 acceptance commits a README image from `opstui shot` while arc 1 only lists `dump::cells/ansi` (the SVG dumper is unscheduled); the theme loader parses `[effects]` in arc 1 while nothing consumes it until arc 3, so retrowave's `startup` sweep is a no-op for two releases; the determinism test replays through 'the full App with TestBackend' but `opstui-app` is written against `DefaultTerminal`, so the loop must be generic over `Backend` and nothing says so.

**Fix:** Honour Borderless/Custom in arc 1 (it is a flag on `theme.block`); produce README images via an external ansi→svg/png step until the SVG dumper is scheduled; ignore-with-warning unknown `[effects]` until arc 3; make `Shell::run<B: Backend>(&mut Terminal<B>)` part of the arc-1 app contract.

