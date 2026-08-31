<!-- Judge verdict. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Judge verdict — performance

PERFORMANCE & ROBUSTNESS: render loop and collector threading (60 fps audio viz beside 1 s NVML polls and a 1000+ row process table), history memory growth, startup time, resize, behaviour when a source fails (no GPU, i2c busy, PipeWire idle, MPRIS player quits), i2c contention with the running root astral-watch logger, and correctness of the concurrency primitives. Verified during judging: astral-watch's Lifecycle exposes only new()/observe() (no active() accessor); i2c.rs re-opens /dev/i2c-N per call behind a process-global ACCESS latch; arc-swap 1.9.2, rtrb 0.4.0 exist; smallvec stable is 1.15.1 (2.0 is alpha).

**Winner under this lens:** opstui — one store, many views (Proposal 3). Under a performance-and-robustness lens the single-writer Store + bounded try_send channel + per-source generation gating + virtual Clock is the design with the fewest concurrency primitives to get wrong, the only one whose sources are catch_unwind-supervised with restart/drop counters, and the only one where a replayed journal reproduces a perf or robustness bug byte-for-byte; its weaknesses (unsafe Ring, per-frame sort in pure views, no render-side catch_unwind) are cheap grafts from patchbay. patchbay ties on total and is the stronger design for render-side containment and service throttling; Proposal 1 loses on silent thread death, lossy alert events and DSP on the render thread.

## Scores (1–10 each; total /60)

| proposal | modularity | performance | extensibility | testability | session velocity | showcase | total |
|---|---|---|---|---|---|---|---|
| opstui — pragmatic core-first (Proposal 1) | 6 | 7 | 6 | 7 | 8 | 7 | **41** |
| patchbay (Proposal 2) | 9 | 8 | 9 | 8 | 5 | 8 | **47** |
| opstui — one store, many views (Proposal 3) | 8 | 8 | 8 | 9 | 6 | 8 | **47** |

### opstui — pragmatic core-first (Proposal 1)

**Strengths**

- Simplest correct threading: one input thread as sole event::read() caller, one std sampler thread per source kind via Feeds::get_or_spawn (so gpu+pins+process table share one NVML/one procfs thread), render thread owns Terminal/App/effects; matches ratatui FAQ and astral-watch's own tui.rs.
- spawn_source is a single 40-line scheduler with Unavailable→exponential backoff to 30 s, Degraded→normal period, interruptible sleep on ctl.stop, and hidden→max(base,1 s) throttling; pins never stops so the alarm overlay works on any page.
- Ctl.detail levels (0 meters / 1 top-N / 2 full columns) gate smaps_rollup/io exactly like htop's PROCESS_FLAG_* — the only proposal with an explicit mechanism for the 130 ms smaps cost.
- Concrete performance budget (tick ≤1 ms, draw ≤4 ms @30 fps / ≤8 ms @60, idle <2% core, audio <8%, RSS <60 MB) and a --stats overlay in arc 1 so VTE throughput is measured on the real 250×70 layout.
- All four failure classes enumerated with concrete states (LibloadingError→smi tier/30 s retry, LibRmVersionMismatch→reboot notice, GpuLost→re-init, PermissionDenied/NoBuses/NoTelemetry for i2c, pw-record missing/exit/passive-idle→silence decay, no session bus).
- Effects deferred to arc 4 and a 1 Hz mtime stat watcher instead of notify: least moving parts in the first two arcs.
- Fixed rings everywhere (HISTORY=300, 120-sample sparklines, 600-sample GPU charts, 8192-frame audio ring, 200-line log) — bounded memory by construction.

**Weaknesses**

- No panic containment anywhere: spawn_source says 'Must not panic' but has no catch_unwind, so a panicking sampler dies silently and the Feed slot keeps its last Sample with status Ok forever; render() is not wrapped either, so one bad Rect/data case takes the whole dashboard down.
- Lifecycle Event::{Raised,Repeated,Resolved} are carried inside PinsSnap through a latest-value slot (Arc<Mutex<Arc<Sample>>>) and turned into AppAlerts by drain_alerts(); if two 500 ms samples land between ticks (render thread stalled by a modal, a slow draw, or a paused source), Raised/Resolved events are lost. Since Lifecycle has no active() accessor (verified), the source must track the active Condition set itself and the overlay should key on that set, not on events.
- Audio DSP runs in tick() on the render thread ('a 2048-point realfft is ~10 µs') and the capture path bypasses Source/Feed entirely (reader thread → Arc<Mutex<Ring>>), so audio is neither recordable nor replayable and a 60 fps stereo FFT + band mapping is charged to the frame budget.
- Dirty tracking is global: every Msg::Wake sets self.dirty = true, so a 250 ms GPU sample for a tile on another page still redraws the whole frame; no per-source generation or per-cell dirty.
- No staleness rendering: nothing dims or badges a tile whose last sample is older than 3× its cadence, which is exactly the visible symptom of the silent-thread-death case above.
- feed.slot.lock().unwrap() on the render thread propagates Mutex poisoning from a sampler panic into a render-thread panic; Relaxed atomics on Ctl are fine but the whole slot design is a Mutex where an ArcSwap would be lock-free.
- MPRIS on zbus::blocking with 'commands arrive on an mpsc::Receiver<PlayerCmd> the source drains each loop' is under-specified: blocking PropertyStream iterators cannot be multiplexed with a Receiver on one thread; needs GetAll polling or a private runtime.
- Arc 1 (core + grid + theme + record/replay/demo + clock/cpu/gpu/pins + CI) is overloaded for one session; the pragmatic design does not translate into a pragmatic first arc.

### patchbay (Proposal 2)

**Strengths**

- Feed<S> is the best-engineered publish/subscribe primitive of the three: ArcSwapOption latest (lock-free reads), AtomicU64 generation for cheap 'did anything change' in tick(), RwLock<Health>, a command channel, and an interest counter with an RAII Interest guard so services throttle or stop (audio child killed after 30 s) when no visible component holds one.
- Per-cell dirty via Msg::Wake(id) plus RedrawPolicy::{OnChange, Animated{fps}} per component: a static page costs zero draws, and only the audio cell asks for 60 fps.
- Render-side robustness is explicit: catch_unwind around each component.render, mark Unavailable('panicked: …'), draw a chip; 'one bad tile cannot take the dashboard down'; STALE 12s badge derived from Feed::generation() age; stderr dup2 before the alternate screen.
- Startup capability probe (≤200 ms, each probe on a thread with a timeout) + manifest requires/optional + placeholder tiles with the fix text ('usermod -aG i2c', udev rule) — the most complete answer to 'no GPU / no i2c / no PipeWire / no D-Bus' as first-class, non-error states; `doctor` subcommand.
- Audio DSP lives on the service thread and publishes instance-agnostic 64-band frames, so the visualizer and the Winamp tile share one FFT and the render thread does only ballistics.
- Polling tiers match the measured NVML costs (250 ms fast, 1 s slow, static once, PCIe from byte counters, never pcie_throughput); pins 500 ms while interested, 1 s otherwise, never stops; solve() cached per (page, body) for resize.
- ServiceDef { start, demo, replay } makes live/synthetic/replayed data indistinguishable to components; hardware-free CI feature matrix.

**Weaknesses**

- Publish ordering hazard: latest (ArcSwap) and generation (AtomicU64) are separate; if generation is bumped before the swap (or without Release/Acquire), a reader can observe the new generation with the old snapshot, mark it consumed, and stay stale until the next publish. Needs 'store snapshot, then bump generation with Release; reader loads generation with Acquire then snapshot'.
- Service threads are not stated to be catch_unwind-protected — spawn_sampler 'converts Err into Health::Degraded with backoff' but a panic in sample() (procfs edge case, NVML UnexpectedVariant path) kills the thread and leaves the Feed frozen with only the STALE badge as evidence; no restart counter.
- TelemetryStore is RwLock<HashMap<MetricKey, Series>> written by every service thread inside publish and read per key by the render thread; correct but it is a second shared-mutable path beside Feed, and Sample { at: Instant, v: f64 } is 24 B on Linux (Instant is 16 B), so the stated '4096 × 400 × 16 B ≈ 30 MB' worst case is ~39 MB.
- PipeWire idle is not handled explicitly: the audio row says node.passive + 'supervisor with 250 ms→5 s backoff' but never states 'no data ≠ dead child; >250 ms silence → decay bars, respawn only on EOF/exit' — the digest's documented trap that makes a passive stream on an idle DAC look like a hang.
- No mechanism for a component to request expensive procs columns (smaps_rollup/io) — 'only when a column needs them' has no channel (Cmd = () for read-only services); interest is boolean, not a detail level.
- MPRIS uses zbus blocking-api with 'property streams (push)' and a per-player thread plus a MediaCmd channel — the same blocking-iterator vs command-receiver multiplexing problem as Proposal 1, unstated.
- 17 crates, Manifest/Registry/CONTRACT_VERSION/Capability/Tier/testkit macros before the first real tile: arc 1 ends with clock + cpu meters only. Cross-crate API churn in every session is a real velocity and review cost, and `cargo test --workspace` grows with each component crate.
- Probe runs Nvml::init() on a probe thread and then the nvml service inits again; harmless (refcounted) but if the driver is wedged the timed-out probe thread leaks a blocked init.

### opstui — one store, many views (Proposal 3)

**Strengths**

- Cleanest concurrency story: the Store is owned by the render thread and mutated only by Store::apply(&Msg); sources try_send into one bounded sync_channel::<Msg>(4096) and count drops; no lock anywhere on the data path; demand is one AtomicU8 per source. The fewest primitives to get wrong, and recording is a channel tee.
- Sources run under catch_unwind with a supervisor that restarts with 250 ms→5 s backoff and a SourceStatus { state, reason, hint, since, last_sample, dropped, restarts } surfaced in a `sources` tile — the strongest source-failure telemetry of the three; LibRmVersionMismatch explicitly not retried.
- Per-source generation dirty gating (dirty() = generation changed for any source a visible component needs, or overlay/effect active, or 1 Hz heartbeat) plus a bounded 3 ms ingest step per frame; process-table records are an Arc swap so a 636–1000 row table costs O(1) to apply.
- Audio: pw-record io thread → rtrb SPSC → dedicated DSP thread publishing Vector Arcs; 60 Hz bands never touch the render thread except as an Arc read; pw-record killed after 5 s hidden; >250 ms no data treated as silence (explicit).
- Virtual Clock: sources, loop and ViewCx all read Ts from one Clock; replay is byte-deterministic (determinism test hashes frames), which makes performance and robustness bugs reproducible from a journal recorded on torch.
- Explicit retention (max_len 2400 / max_age 10 min) with a 32 MB hard cap, Vector series with short history only, Records latest-only; alert log ring of 500; AlertLog keyed on AlertId transitions with hold/clear hysteresis so the banner cannot flicker.
- All four lens failure cases handled with concrete states (§8), pins source never paused (≥500 ms hidden), TelemetryLost fed to Lifecycle on implausible reads, ICMP EPERM → TCP-connect probes, RAPL 0400 → absent key with hint.

**Weaknesses**

- Pure view(&self, cx) with an immutable ui: &dyn Any forces the htop tile to re-sort/filter 1000+ ProcRows every rendered frame (up to 60 fps while the audio tile is focused): ~100–150 µs per frame plus allocation, or an interior-mutability memo keyed on generation that quietly breaks the 'pure' claim. Should be an explicit per-generation derive step.
- Ring<T> { buf: Box<[MaybeUninit<T>]>, head, len } is hand-written unsafe for a fixed-capacity ring — a self-inflicted soundness risk where VecDeque::with_capacity + pop_front (or Box<[T]> with T: Copy + Default) is equivalent and safe.
- No component-side panic containment: relies on a proptest that every component renders every Rect ≤12x6 without panicking; a data-dependent panic in view() (odd MPRIS metadata, NaN in resample) still kills the UI. Per-cell catch_unwind (Proposal 2) is missing.
- solve_layout(term.size()?) runs on every loop iteration (every message and every frame) rather than cached per (page, body); cheap (<50 µs claimed) but wasteful at 60 Hz audio message rates, and term.size() is a syscall.
- Key::named(&str) builds Label::Name(Arc<str>) per call — components calling net::RX_BPS.named('eno1') inside view allocate every frame; resample allocates a Vec<Option<f64>> per series per frame. Fine at 30 fps, sloppy at 60.
- RuleEngine::observe runs inside Store::apply for every batch including 60 Hz audio batches; needs a name-indexed rule map or it is O(rules × touched) per batch; borrow-splitting apply(&mut self) vs observe(&Store) is an implementation trap.
- Ingest pause (`space`) lets the channel fill to 4096 messages (dropped thereafter) and then applies 40 s of stale batches on resume, including old rule evaluations; no 'drain and keep latest per key' on resume.
- The store crate (Key/Datum/Series/resample/RuleEngine/journal/catalogue) must exist before any tile renders, and every new metric touches three places (keys, source, demo::Synth); arc 1 is as overloaded as Proposal 1's.

## Best ideas from the non-winners

- patchbay: Feed::acquire() -> Interest RAII guard + interested() so services throttle/stop (kill pw-record after N s, drop procs to 3 s) purely from what is on screen; keep Proposal 3's Level atomic but derive it from interest counts.
- patchbay: catch_unwind(AssertUnwindSafe) around every component render (and on_key), mark the instance Unavailable('panicked: …'), draw a placeholder chip, stop calling it after the first panic — combine with Proposal 3's source-side catch_unwind + restart counters so both halves are contained.
- patchbay: 'STALE 12s' badge computed from generation/last_sample age > 3× cadence, dimming the tile; every proposal's silent-thread-death case becomes visible.
- patchbay: startup CapSet probe with per-probe timeouts, manifest requires/optional lists, placeholder tiles carrying the fix text, and a `doctor` subcommand; keep it from double-initialising NVML (probe result handed to the service).
- patchbay: per-cell dirty (Msg::Wake(id)) and RedrawPolicy::Animated{fps} per component; cache solve() per (page, body) and recompute demand only on layout change.
- patchbay: ServiceDef { start, demo, replay } triple as the single seam — Proposal 3 already has demo::Synth and Replay as Sources; make the triple mandatory per source so audio and MPRIS are replayable too (Proposal 1's audio bypasses its Feed entirely).
- Proposal 1: Ctl.detail levels (0 meters / 1 top-N / 2 full columns) so the procs source only reads smaps_rollup/io/cgroup at level 2 — the 130 ms smaps cost must be gated by column visibility, not just by interest.
- Proposal 1: defer tachyonfx to a later arc and ship the --stats/F12 frame-time + changed-cells + bytes-written overlay in arc 1; decide 60 fps opt-in only after measuring VTE on the real 250×70 layout.
- Proposal 1: 1 Hz mtime stat watcher for config/theme reload (immune to editor rename tricks, no notify rc dependency) — good enough until edit-mode save exists.
- Proposal 1: Feeds::get_or_spawn keyed by source kind so multiple instances (net-lan, net-wifi) share one sampler thread and one NVML handle; and deny.toml bans on cpal/mpris/pipewire/libpulse (drop the tokio ban if Proposal 3's single async thread is kept).
- Proposal 1: own 30-line ▀ halfblock painter instead of ratatui-image (fewer deps, no from_query_stdio tty race); pre-decode art on the source thread.

## Concerns for the synthesis

- Panic containment on BOTH sides: source threads under catch_unwind with backoff restart and a restart counter (Proposal 3), and per-cell catch_unwind around component render/on_key with an Unavailable chip (Proposal 2). No proposal has both; a 24/7 dashboard beside a running game needs both.
- Alert events must never travel through a latest-value slot (Proposal 1's PinsSnap.events). Route Lifecycle events as channel messages, and key the overlay on the active Condition set / AlertId transitions. astral-watch's Lifecycle has no active() accessor (verified in src/lifecycle.rs), so the pins source must maintain the active set from events itself.
- If a Feed/generation design is kept (Proposal 2), specify the ordering: store the snapshot, then bump generation with Release; readers load generation (Acquire) before the snapshot, and re-check after. Alternatively adopt Proposal 3's single-writer Store and avoid the question.
- Process table derived state (sort, filter, tree, selection-by-PID) must be recomputed once per source generation, not once per rendered frame; Proposal 3's pure-view contract needs an explicit per-instance derive/tick step or the htop tile pays ~100–150 µs per frame at 60 fps.
- No hand-written unsafe ring buffers (Proposal 3's Ring<MaybeUninit<T>>); use VecDeque with a fixed cap or rtrb (already in the tree for audio). Run miri in CI if any unsafe survives.
- Audio supervisor semantics must be explicit: respawn only on child EOF/exit, never on 'no data' (node.passive on an idle sink delivers nothing); >250 ms without data → decay bars; DSP on its own thread, not the render thread; a 'starting' state while pw-record respawns after an interest gap.
- MPRIS threading: blocking zbus property streams cannot be multiplexed with a command receiver on one thread. Either a private current_thread tokio runtime inside the mpris thread with select! (Proposal 3), or GetAll polling every 500 ms plus NameOwnerChanged, with zbus built default-features=false + tokio to avoid an async-io duplicate.
- Staleness is a first-class render state: every source publishes last_sample; the shell dims and badges tiles whose age exceeds 3× cadence; the `sources` diagnostic tile (Proposal 3) shows state/cadence/age/dropped/restarts.
- Bounded data channel with try_send + drop counter (Proposal 3), sized so a 60 Hz audio publisher plus 250 ms GPU plus 1.5 s procs cannot fill it in under ~30 s; on resume from pause, drain to latest-per-key rather than replaying 4096 stale batches through the rule engine.
- Memory caps must be explicit for every history: scalar rings (retention max_len/max_age, ≤32 MB total), vector series (short), records latest-only, alert log, toast queue, audio ring (8192 frames/ch), art cache (≤8 × 256 px). Store sample timestamps as u64 ns, not Instant (24 B per sample).
- Startup path: NVML init, detect_bus (bytewise probe of up to 7 NVIDIA buses can take hundreds of ms), pw-record spawn and D-Bus connect all off the render thread; first frame < 300 ms with placeholder tiles; capability probe time-boxed and its result handed to services so NVML is not initialised twice.
- i2c contention: keep ≥500 ms cadence on the block-read path, treat EIO/timeouts as TelemetryLost (Lifecycle freeze), prefer the exporter when 127.0.0.1:9942 answers, and tell the user the running root logger (PID 6755, built before the block-read commit) is on the 36-transaction bytewise path — restarting it from HEAD cuts bus time ~8×.
- Dirty gating must be per-source generation or per-cell, never a global flag set by any Wake (Proposal 1); sources feeding only hidden pages must not cause redraws; audio at 60 fps must only invalidate its own cell.
- Resize: rely on ratatui autoresize but cache solve() per (page, body) and recompute demand/interest levels on size or page change; the too-small stack mode must be reachable from any page without losing focus state.
- Arc-1 scope: all three overload the first session. Cut arc 1 to core + grid + theme + clock + cpu (meters, no table) + the stats overlay, with record/replay/demo, and measure VTE throughput before committing to 60 fps or a fourth component.
- Avoid .lock().unwrap() on the render thread (poisoning from a sampler panic becomes a UI panic); use unwrap_or_else(PoisonError::into_inner), parking_lot, or lock-free ArcSwap/single-writer.
- Rule engine (if kept from Proposal 3) needs a name-indexed rule map so 60 Hz audio batches cost O(touched) not O(rules × touched), and NaN/absent handling in Cond so a missing metric cannot raise or clear an alert.
