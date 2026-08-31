<!-- Judge verdict. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Judge verdict — maintainability

MAINTAINABILITY & MULTI-SESSION VELOCITY — judged as the principal engineer who maintains this alone for years in one-session arcs with adversarial review before each commit: crate boundaries, abstraction cost vs benefit, cost to add/remove a component, config ergonomics, and how much a future session can do without re-reading everything. Technical claims were spot-checked against the research digest, cargo info (enum-map 3.1.0 rust-version 1.95, enum-map 2.7.3 1.61, zbus 5.19 1.87, hickory-resolver 0.26.1 1.88, notify-debouncer-full 0.8.0-rc.2 1.88, proptest 1.85, criterion 0.8.2 1.86) and astral-watch's src/tui.rs (chained panic hook at lines 509-511 that runs DisableMouseCapture).

**Winner under this lens:** patchbay — under a maintainability lens the contract (Manifest + ComponentDef + Registry, ServiceDef with mandatory start/demo/replay, Feed with interest, tiers, capability probe with fixes, effects-as-data, docs and testkit) is the one I would still be glad to own in year three; it is the only proposal where audio/MPRIS are replayable by construction and where a reviewer can verify component isolation mechanically. Its two costs — nineteen crates and an infrastructure-only arc 1 — are fixable in synthesis by adopting the store proposal's five-crate layout and the pragmatic proposal's arc-1 ambition (trimmed), whereas the pragmatic proposal's per-component history and audio side-channel, and the store proposal's pure-view/closed-Command constraints, are structural.

## Scores (1–10 each; total /60)

| proposal | modularity | performance | extensibility | testability | session velocity | showcase | total |
|---|---|---|---|---|---|---|---|
| opstui (pragmatic core-first, single package) | 6 | 7 | 6 | 7 | 7 | 8 | **41** |
| patchbay (contract-first workspace) | 9 | 8 | 9 | 8 | 6 | 7 | **47** |
| opstui ('one store, many views', data/reactive-first) | 8 | 8 | 7 | 9 | 5 | 7 | **44** |

### opstui (pragmatic core-first, single package)

**Strengths**

- Smallest conceptual surface: 'exactly three traits/types carry the design: Component, Source, Theme' — a future session can hold the whole contract in its head, and the 9-step 'Adding a component' checklist plus the 90-line clock.rs template is the most concrete onboarding path of the three.
- Source/spawn_source is a genuinely good 40-line seam: one loop gives every poll-style sampler backoff, visibility-based cadence, JSONL recording and replay ('components cannot tell live from replay from demo'), with a mandatory demo: fn(u64) -> Snap in get_or_spawn so demo mode can never lag behind.
- Dependency policy is CI-enforced rather than advisory: 'deny.toml bans LGPL/NC licences, tokio, cpal, mpris, pipewire, libpulse*' turns the digest's do-not-use findings into build failures — cheap and durable across sessions.
- Hot reload as a 1 Hz mtime stat thread ('No notify dependency: one stat per second is free and immune to editor rename tricks') avoids the directory-watch/debounce ceremony and a release-candidate crate; edit-mode save via toml_edit is correctly deferred.
- Arc 1 ends screenshot-worthy (cpu + gpu + pins in retrowave) and ships the --stats overlay first, so the VTE throughput question is measured on the real 250x70 layout in the first session.
- Correct on the hard verified facts: 24-col grid with real-Rect size classes, PCIe from byte-counter fields never pcie_throughput, ratatui-image avoided in favour of a 30-line halfblock painter, astral-watch pinned by rev with default-features=false and [patch] to the sibling checkout, stderr dup2'd before the alt screen.

**Weaknesses**

- Audio silently escapes the one abstraction that justifies itself: 'a reader thread pushes f32 frames into Arc<Mutex<Ring>>' and 'DSP runs in tick() on the render thread' — so the audio and Winamp components are the only ones that cannot be recorded, replayed or demoed through Feed, exactly the components hardest to develop without the machine. The Source trait is poll-only, which is also why MPRIS ends up as a 1 s poll loop that drains a command receiver rather than reacting to property streams.
- History is per-component ('The component keeps HISTORY = 300 per-pin rings, peaks, watts ring and a 200-line log'; cpu/gpu/net each push their own rings in tick()). Every new component re-implements compare-seq/push-ring/resample-to-width, and two components showing the same metric can disagree — a maintenance tax the other two proposals eliminate with a shared store.
- Feeds is stringly typed: HashMap<&'static str, Box<dyn Any + Send + Sync>> with get<T>(name) downcasts; the player→audio cross-read depends on a string agreed by convention, and a typo is a runtime None, not a compile error.
- Dirty gating is coarse: spawn_source sends Msg::Wake on every sample and the loop does `Ok(Msg::Wake) => self.dirty = true`, so a hidden 250 ms GPU tick redraws pages that do not show it; the per-source generation gating in the store proposal is strictly better and no harder.
- Single crate means nothing is enforced: a component can call astral_watch::i2c or spawn a thread and only review catches it; incremental compile time grows with every arc (astral-watch alone brings 70 crates into this one package). No docs/ directory — the checklist exists only in this proposal.
- Arc 1 is overscoped for one session plus adversarial review: full grid/theme/source/replay/demo/record infrastructure AND four components including per-CCD core bars, a process table, NVML fast/slow tiers with a spec table and nvidia-smi fallback, and the full pins panel with Lifecycle-driven alarm overlay. Realistically this is two arcs.
- No capability model: degraded states are per-source strings, so there is no startup 'doctor' view and no way for a manifest to say 'needs i2c group' before the tile is built.

### patchbay (contract-first workspace)

**Strengths**

- The contract is the product and it is complete: Manifest (footprints, requires/optional capabilities, services, example_options, keys) + ComponentDef + Registry + BuildCtx + object-safe Component with tiers() and RedrawPolicy, and ServiceDef { start, demo, replay } — demo and replay are mandatory per service, so unlike the single-package proposal, audio and MPRIS are replayable by construction ('components never know whether data is live, synthetic or replayed').
- Crate boundaries enforce the rules a reviewer would otherwise have to check by hand: 'Service crates depend on core plus their system crates, never on ratatui. Component crates depend on core, ratatui-core/-widgets and the service crates they consume, never on the app.' Effects stay as data in core ('EffectSpec is plain data; only patchbay-app maps it onto tachyonfx'), so component crates never pull tachyonfx.
- Capability model with actionable degradation: CapSet probe ≤200 ms with per-probe timeouts, `patchbay doctor`, and placeholder tiles that show 'the reason and the fix (apt install …, udev rule …, usermod -aG i2c)'. This is the best answer on the panel to the digest's list of things missing on torch.
- Feed<S> is the right primitive: ArcSwapOption latest + generation + health + typed Cmd channel + RAII Interest so services throttle when nobody is looking; cross-component sharing goes through feeds (media reads audio, gpu and pins both read nvml) not an event bus.
- Extensibility is honest and staged: static registration via `patchbay_app::run(Registry, Cli)` so a third party builds their own binary; cdylib rejected with reasons; an `exec` (JSON-lines) component planned before any WASM; CONTRACT_VERSION checked at registration.
- Multi-session tooling is explicit: docs/ARCHITECTURE.md and ADDING-A-COMPONENT.md, `patchbay component info <kind>` generated from manifests, a testkit crate with snapshot_matrix! and assert_never_panics, committed 60 s recordings, headless --screenshot in CI, and a per-feature CI matrix.
- Data seam is sound for performance: TelemetryStore holds scalars only with bounded caps and Series::resample so charts are tick-rate independent; snapshots keep detail; audio publishes 64 instance-agnostic bands so the visualiser and Winamp tile never run a second FFT.

**Weaknesses**

- Nineteen crates for one person is real ceremony: adding a component touches a new crate, the cli feature list, builtin_registry(), the CI feature matrix and docs — five places — and every crate carries its own Cargo.toml/lib.rs/register boilerplate. The boundaries are worth keeping; the crate count is not.
- The crash-containment claim is wrong as written: 'a component that panics is caught per frame (catch_unwind around render)' combined with 'Mouse capture and focus events are undone in a chained panic hook exactly as astral-watch does'. Rust runs the panic hook before unwinding, so ratatui's init hook restores the terminal and astral-watch-style hooks (tui.rs:509-511) disable mouse capture — the dashboard survives the panic in cooked mode on the main screen. It needs a custom hook gated on a thread-local 'inside component render' flag, or the claim must go.
- MSRV slip: Theme uses `enum_map::EnumMap<GradientId, Gradient>`; enum-map 3.1.0 declares rust-version 1.95, which breaks the 1.88 MSRV job unless enum-map is pinned to the 2.x line (2.7.3 is 1.61). Small, but exactly what an MSRV CI job exists to catch.
- Two homes for data: components read `feed.latest()` for detail and `store.with(key, …)` for history, and MetricKey(&'static str) is an untyped string — the store proposal's typed Key<T> catalogue is the cleaner version of the same idea.
- Arc 1 is infrastructure-heavy with little to show: core 'complete', app with notify hot reload, doctor, headless screenshots, testkit, proptests — and only clock + cpu (three tiers, no process table). That is a solid foundation but the first session ends with a CPU tile, which weakens the implement→review→approve rhythm the user wants.
- 12-column grid diverges from the digest's 24-column plan without saying why; with 12 columns a 1x1 on 250x70 is ~20 cells wide, so the 1x1 'badge' tiers are coarser than the size-class plans in the research assumed.
- Bus/BuildCtx/Services/Interest/Tier/Health/Capability/Manifest/ComponentDef/ServiceDef/Feed/TelemetryStore is a lot of small types to learn before writing a component; the docs mitigate this but the first read of core will take a session.

### opstui ('one store, many views', data/reactive-first)

**Strengths**

- Best testability story on the panel: single-writer Store::apply(&Msg) is a pure state transition; a virtual Clock and `cx.now` make 'rendered under replay at a fixed Ts … byte-for-byte deterministic'; snapshots use an ANSI dump that carries colours (TestBackend's Display does not); input is tested as 'keys in, commands out'; a determinism test hashes frames across two replays; screenshots are regenerated in CI and fail on drift.
- Typed metric catalogue is a durable multi-session asset: Key<T> constants in keys/*.rs shared by sources, components and rules, plus `opstui keys` generating docs/keys.md in CI 'so it cannot drift'. A new view over existing data is 'steps 2-4 only … about a hundred lines and a snapshot'.
- Five-crate layout with strict direction is the right size for a solo workspace: `store ← ui ← components ← bin`, `store ← sources ← bin`, and 'opstui-store compiles in about a second with no TUI or system dependencies' — fast headless iteration on the logic that matters.
- Performance model is the tightest: no locks on the data path, per-source generation dirty gating (`store.generation(src) != last_gen[src]` only for sources a visible component needs), demand Level {Hidden, Visible, Focused} with Cadence per source, bounded ingest per frame, Arc-swapped ProcTable records.
- Side effects are isolated by construction: components return Command values executed on an executor thread, so kill/renice/MPRIS calls can never block a frame and are trivially testable.
- Correctly supports async-native crates without infecting the core: AsyncSource on one current_thread tokio behind mpris/net-probe features, while everything blocking stays on std threads.

**Weaknesses**

- Highest concept load of the three: Ts/Clock/Key/Label/Datum/Value/Ring/ScalarSeries/VectorSeries/RecordSeries/Msg/Batch/Source/AsyncSource/SourceCtx/Cadence/Level/Control/Command/Outcome/ViewCx/RuleEngine/Journal/Executor. A future session must understand store semantics before touching anything; 'session velocity' for a new source is four crates (keys in store, module in sources, demo::Synth, component + registry, possibly Command in ui).
- The pure-view contract has no per-frame mutable state: Component has no tick(), ViewCx carries `ui: &dyn Any` immutably, and per-instance state is a Box<dyn Any> downcast on every call. Winamp peak-fall ballistics, htop new/tomb row highlighting and DSP presets therefore migrate into the sources ('attack/release EMA, selectable winamp/cava gravity + peaks' in the audio source) or must be re-derived from history each frame — friction that lands exactly in the audio/winamp/htop arcs.
- `Command` is a closed enum in opstui-ui listing Kill/Renice/Affinity/IoPrio/Media/AudioTarget/…: every component with a new action edits the ui crate, the opposite of the component-local `S::Cmd` the patchbay proposal uses.
- Unnecessary unsafe: `Ring<T> { buf: Box<[MaybeUninit<T>]> … }` where VecDeque::with_capacity gives the same O(1) push; this adds a miri obligation to the crate that is supposed to be the easy one to review.
- Determinism is asserted but not designed: Store keeps HashMaps and `labels(name)` iterates them, so any view that enumerates interfaces/cores/pins gets randomized order unless BTreeMap or sorting is mandated; `MetricId.named(&str)` allocates an Arc<str> per lookup per frame.
- Scope creep in arc 1: rules.toml is a small expression language ('when = ">= gpu.slowdown_c - 5"', label wildcards `{nvme*}`, `absent 5s`) plus hold/clear hysteresis, JSONL journal, ANSI and SVG dumpers, full cpu and gpu sources, four components, supervisor with catch_unwind, determinism test and a perf doc — larger than either competitor's arc 1.
- Same panic-hook flaw as patchbay: 'Sources run under catch_unwind; the render thread installs color-eyre's hook chained after ratatui's restore' — a caught source panic still fires the global hook and restores the terminal from a source thread.
- Small inaccuracies that a reviewer would flag: serde_json 'already in the tree via astral-watch' is false in arc 1 (pins is deferred to arc 2); `EnumMap<GradientId, Gradient>` hits the enum-map 3.x MSRV 1.95 problem too.

## Best ideas from the non-winners

- From the store proposal: the typed Key<T> metric catalogue in a headless store crate with `opstui keys` generating docs/keys.md in CI — replace patchbay's MetricKey(&'static str) with it, and make Series::resample the single history API so no component keeps private rings.
- From the store proposal: virtual Clock/Ts threaded through sources, the loop and the render context, so replay and snapshot tests are byte-deterministic; pair it with the ANSI buffer dump for colour-carrying snapshots instead of TestBackend's text-only Display.
- From the store proposal: per-source generation dirty gating (redraw only when a source a visible component needs has advanced) and the three-level demand (Hidden/Visible/Focused) with per-source Cadence, plus the `sources` debugging tile that lists status/cadence/age/drops.
- From the store proposal: the five-crate shape (store/ui/sources/components/bin) with feature-gated modules inside `sources` and `components` — keep patchbay's contract and dependency direction, drop the one-crate-per-component ceremony.
- From the store proposal: side effects as Command values run on an executor thread, tested as keys-in→commands-out — but keep patchbay's per-service typed `Cmd` (or a `Box<dyn Any>` escape hatch) instead of a closed enum in the ui crate.
- From the store proposal: AsyncSource on a single current_thread tokio behind features for zbus/surge-ping/hickory, so ICMP probes and reverse DNS are not permanently ruled out by a tokio ban.
- From the pragmatic proposal: deny.toml that bans LGPL/NC licences and the crates the research verified cannot build here (cpal, mpris, pipewire, libpulse*), so a transitive pull fails CI rather than review.
- From the pragmatic proposal: the 90-line clock.rs template plus a numbered 'adding a component' checklist checked into docs, and a `--stats` frame-time/changed-cells/bytes overlay shipped in arc 1 so VTE throughput is measured on the real layout.
- From the pragmatic proposal: a 1 Hz mtime-stat config watcher (no notify/debouncer dependency, immune to editor rename) as the arc-1 hot reload, and the in-tree ▀ halfblock painter instead of ratatui-image.
- From the pragmatic proposal: arc-1 ambition — end the first session with a screenshot-worthy Overview (cpu + gpu at least) in retrowave, not a lone CPU tile; keep patchbay's infrastructure but cut doctor/hot-reload/headless-screenshot to arc 2 to make room.

## Concerns for the synthesis

- catch_unwind does not contain panics the way patchbay and the store proposal claim: Rust runs the panic hook before unwinding, so ratatui's init hook restores the terminal and an astral-watch-style chained hook (tui.rs:509-511) disables mouse capture. Either install a custom hook that consults a thread-local 'in component render / in source thread' flag and only restores when the render thread itself is dying, or drop the containment claim and let a panicking source thread report Health::Unavailable via a JoinHandle result instead of catch_unwind.
- MSRV: enum-map 3.1.0 requires Rust 1.95; pin `enum-map = "2"` (2.7.3, MSRV 1.61) or use a plain array indexed by a #[repr(u8)] enum. Also confirm hickory-resolver 0.26.1 (1.88) and notify-debouncer-full 0.8.0-rc.2 (1.88) stay at or below the chosen 1.88 floor; prefer notify 8.2 + debouncer 0.7 or the mtime poller.
- Crate count: keep patchbay's dependency direction but collapse to ~6 crates (core/store, ui, sources, components, app, cli, testkit) with Cargo features selecting modules; require that `cargo tree -d` and a feature-matrix job still prove single-versioned ratatui-core/crossterm and header-free builds.
- Every proposal overscopes arc 1. Define arc 1 as: contract + grid + theme loader (modern/retrowave/mono) + Source/Feed/spawn + demo/replay/record + cpu and gpu components at 2-3 tiers each + snapshot matrix + --stats overlay + CI. Push pins, hot reload, doctor, edit mode, rules engine, SVG dumps and process actions to arcs 2+.
- One home for history: components must not keep private rings (pragmatic proposal) and must not need two APIs (patchbay's Feed.latest vs TelemetryStore). Snapshot = latest detail; store = scalar history with resample; both fed on publish and on replay.
- Push-style sources (pw-record audio, zbus MPRIS) must go through the same feed/record/replay seam as poll sources; the pragmatic proposal's Arc<Mutex<Ring>> side channel and render-thread DSP must not survive synthesis.
- Per-frame animation state needs a defined home: give Component a `tick(&mut self, ctx) -> Redraw` (patchbay) or an explicit mutable animation-state slot; the store proposal's pure `view` with immutable `ui: &dyn Any` makes Winamp peak-fall, htop tomb rows and DSP presets awkward.
- Determinism hygiene if replay snapshots are adopted: BTreeMap (or sorted) enumeration of labels/instances, no HashMap iteration in views, all time from ctx.now, seeded RNG only; add a determinism test early so it is enforced, not hoped for.
- Do not write unsafe for a ring buffer; use VecDeque with a fixed capacity. If unsafe appears anywhere, add a miri job (the toolchain has miri).
- Command/control extensibility: use per-service typed Cmd (Feed::send) or a Box<dyn Any + Send> domain escape hatch; a closed Command enum in the ui crate will be edited by every component arc.
- Grid geometry: pick 24 columns (matches the digest's footprint math of ~10-cell units on 250x70) unless there is a documented reason for 12; make rows='auto' and cell_aspect explicit, and validate the 1x1/2x1/4x2/6x3 tiers against the real Ptyxis cell size in arc 1.
- astral-watch integration prerequisites for the pins arc: ask upstream for a v0.8.0 tag plus `cli`/`notify` feature gating and a log facade before pinning; keep `[patch]` in a git-ignored .cargo/config.toml; dup2 stderr before entering the alternate screen in all cases.
- Reconcile the tokio ban (pragmatic deny.toml) with AsyncSource: allow tokio only behind the mpris/net-probe features and assert with cargo-deny that the default feature set has no tokio.
- Snapshot colour coverage: adopt the ANSI dump for insta snapshots or mandate explicit cell fg/bg assertions per theme; TestBackend's Display alone would let a theme regression through.
