> **Status: planning baseline, 2026-08-30.** Written by the design workflow (3 proposals → 3 judges → synthesis → 2 adversarial critics → revision 2) reviewed by Claude, and walked through with Matt on 2026-08-31 — D35 resolves every open decision, so the baseline stands approved for build. Provenance in `docs/design-review/`, evidence in `docs/research/`. Change decisions through `docs/DECISIONS.md`; keep this file current per arc.

# gridwatch — architecture

## 1. Overview

gridwatch is a modular, themeable ops dashboard TUI for the Linux workstation "torch", written in Rust (1.88, edition 2024) on ratatui 0.30.2 + crossterm 0.29. It renders a fixed unit grid of *components* (htop, nvtop, astral-watch pins, network, audio visualizer, Winamp-style MPRIS player, sensors), each of which looks right from a 1x1 tile to full screen, in file-defined *themes* (retrowave, modern, phosphor, terminal, mono), with an in-app edit mode that rearranges the grid and writes the layout back without destroying comments.

The design is the contract-first one (static `Manifest` + `ComponentDef` + `Registry`, components that only draw their inner `Rect`, capability probing, cumulative tiers with minimum sizes, effects as data, a 12-column grid that survives measurement on this terminal) on top of a single-writer typed telemetry **Store** with a virtual clock, a typed `Key<T>` catalogue, per-source generation dirty gating, demand levels and journal record/replay. Revision 2 applies the second review: serialisable records, three channels (input, control, data) so alarms can never be dropped, layout thresholds derived from the grid instead of a constant, cumulative tiers with a defined `view` fallback, optional sources, a two-file config with singleton sources, `Synth` in the store crate, no crossterm below the app crate, and a first arc cut roughly in half. Every finding is resolved explicitly in §16.

## 2. Principles

1. **One vocabulary.** Every metric is a typed `Key<T>` constant in the store crate; sources write it, components read it, alert rules evaluate it, `gridwatch keys` documents it.
2. **Single writer.** The render thread owns the `Store`; the only mutation is `Store::apply(&Msg)`. Sources publish through channels. No lock on the data path, no ordering hazards, recording is a channel tee.
3. **Control is never lost.** Status, alerts, action results and reloads travel on an unbounded control channel drained before data; only telemetry batches are lossy, and drops are counted.
4. **Components describe, never do.** `view(&self)` reads the store and returns a semantic view tree; the theme's renderer draws it; side effects are returned as `Command`s and executed elsewhere. Components never name a `Color`, a glyph, a thread, or a device — and never write a cell except inside `View::Custom`.
5. **Data is replaceable.** Live, demo (seeded `Synth` in the store crate) and replay (journal) are indistinguishable to components. Hardware-free CI, deterministic snapshots and README screenshots fall out of this seam.
6. **Missing is a state, not an error.** No GPU, no i2c group, no PipeWire, no D-Bus: the tile shows the reason and the fix. Panics are contained per source thread and per component render.
7. **Real size, not nominal size.** Tiers are chosen from the inner `Rect`; footprints are only picker hints; every component's lowest tier fits the grid's minimum unit.
8. **Std threads by default; async only where a crate demands it** (zbus, surge-ping, hickory) on one feature-gated `current_thread` tokio thread inside the sources crate.
9. **Small surface per session.** Six crates with strict dependency direction; one checklist to add a component; `docs/` generated from manifests and keys; a parity document per emulated tool.

## 3. Crate layout (summary; full tree in the workspace document)

```
gridwatch-store      ← no TUI crate, no system deps: Ts/Clock, Key<T>, RecordValue + catalogue, Store, Msg + channels,
                     InputEvent, Source/Demand, journal, alerts, demo::Synth
gridwatch-ui         ← ratatui-core/-widgets: Component, Manifest, Registry, Theme, Tier, layout engine, widgets, overlay, dumps, testkit
gridwatch-sources    ← store + system crates, feature-gated modules: cpu, gpu, pins, net, audio, mpris, sensors
gridwatch-components ← store + ui, feature-gated modules: clock, sources, alerts, htop, gpu, pins, net, audio, winamp, sensors
gridwatch-app        ← ratatui facade + crossterm 0.29 + tachyonfx: shell, `pub fn run<B: Backend>(…)`
gridwatch            ← binary: feature flags → registry assembly → gridwatch_app::run
```

Direction: `store ← ui ← components ← app ← bin` and `store ← sources ← app`. Components never depend on sources; record types and the demo generator live in the store. crossterm exists only in `gridwatch-app` and the binary: the store defines its own `InputEvent`, converted once in the input thread. `gridwatch-store` builds in about a second and holds most of the logic that needs tests.

## 4. Core types and traits

### 4.1 Time, identity, records, catalogue (`gridwatch-store`)

```rust
pub struct Ts(pub u64);                                   // ns since run/journal epoch; monotonic, Copy, serde
pub enum Clock { Real { start: Instant }, Virtual(Arc<AtomicU64>) }

pub struct MetricId { pub name: &'static str, pub label: Label }
pub enum Label { None, Index(u16), Name(Arc<str>) }       // core=7, pin=3, gpu=0, iface="eno1", "k10temp:Tccd1"
pub struct Key<T> { pub id: MetricId, _t: PhantomData<fn() -> T> }
impl<T> Key<T> { pub const fn new(name: &'static str) -> Self; pub fn idx(&self, i: u16) -> Self; pub fn named(&self, s: &Arc<str>) -> Self; }
pub type Vec32 = Arc<[f32]>;

/// Every Record type implements this (blanket impl for `T: Any + Send + Sync + Debug + Serialize`).
pub trait RecordValue: Any + Send + Sync + Debug { fn as_any(&self) -> &dyn Any; fn to_json(&self) -> serde_json::Value; }
pub enum Datum { Scalar(f64), Vector(Vec32), Record(Arc<dyn RecordValue>) }

pub struct KeyMeta { pub name: &'static str, pub unit: Unit, pub kind: DatumKind, pub source: SourceId, pub doc: &'static str,
                     pub decode: Option<fn(serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError>> }
pub static CATALOGUE: &[&[KeyMeta]];                       // one slice per keys/<domain>.rs
pub fn lookup(name: &str) -> Option<&'static KeyMeta>;     // interns journal names onto the static catalogue
pub struct SourceId(pub &'static str);                     // constants live next to their keys: keys::cpu::SOURCE, keys::gpu::SOURCE …
```

`f64` keys keep bounded history; `Vec32` keys keep latest plus a short ring (audio bands, NVML 20 ms power trace); `Record` keys keep latest only (process table, GPU static info, now-playing). Every Record type is `Serialize + Deserialize` and registers its `decode` in the catalogue, so the journal can round-trip it and `&'static str` names are never leaked: an unknown name in a journal is skipped with one warning. GPU keys carry a device label from day one (`gpu.util_pct{0}`) so multi-GPU never needs a breaking catalogue change. `gridwatch keys` prints the catalogue; CI regenerates `docs/KEYS.md`.

### 4.2 Store, messages, channels

```rust
pub struct Ring<T> { buf: VecDeque<T>, cap: usize }       // safe; no unsafe anywhere in the workspace
pub struct Retention { pub max_len: usize /* 2400 */, pub max_age: Duration /* 10 min */ }
pub struct Sample { pub id: MetricId, pub datum: Datum }
pub struct Batch { pub source: SourceId, pub at: Ts, pub samples: Vec<Sample> }
pub enum ControlMsg { Status(SourceId, SourceStatus), Alert(AlertEvent), Done(ActionId, Result<String, String>), Reload(Reload) }
pub enum InputEvent { Key(KeyEvent), Mouse(MouseEvent), Resize(u16, u16), Paste(String), FocusGained, FocusLost }   // serde mirror, no crossterm
pub enum Msg { Batch(Batch), Control(ControlMsg), Input(InputEvent), Heartbeat }

pub struct Channels { pub data: SyncSender<Batch> /* bounded 4096, try_send */, pub control: Sender<ControlMsg> /* unbounded */, pub input: Sender<InputEvent> }

impl Store {
    pub fn apply(&mut self, msg: &Msg) -> SmallVec<[AlertEvent; 2]>;      // the only mutation
    pub fn last(&self, k: &Key<f64>) -> Option<(Ts, f64)>;
    pub fn window(&self, k: &Key<f64>, span: Duration) -> impl Iterator<Item = (Ts, f64)> + '_;
    pub fn resample(&self, k: &Key<f64>, span: Duration, buckets: usize, agg: Agg, out: &mut Vec<Option<f64>>);
    pub fn vector(&self, k: &Key<Vec32>) -> Option<(Ts, &Vec32)>;
    pub fn record<T: RecordValue>(&self, k: &Key<T>) -> Option<(Ts, &T)>;   // downcast via as_any
    pub fn labels(&self, name: &'static str) -> impl Iterator<Item = &Label>; // BTreeMap order
    pub fn status(&self, s: SourceId) -> &SourceStatus;
    pub fn generation(&self, s: SourceId) -> u64;
    pub fn last_sample(&self, s: SourceId) -> Option<Ts>;
    pub fn alerts(&self) -> &AlertLog;
}
```

The frame loop drains the three receivers in a fixed order — `input` (all), `control` (all), `data` (at most 3 ms) — building a `Msg` for each and teeing it to the recorder. `resample` writes into a caller-owned buffer and is the single history API. Timestamps are 8 bytes, so a scalar sample is 16 bytes: 200 series × 2400 points ≈ 8 MB, hard-capped at 32 MB by retention.

### 4.3 Sources and demand

```rust
#[repr(u8)] pub enum Level { Paused = 0, Hidden = 1, Visible = 2, Focused = 3 }
#[repr(u8)] pub enum Detail { Meters = 0, Table = 1, Columns = 2 }   // what the richest visible tier needs from a source: meters only, a pid-level process scan, or per-column gated files
pub struct Demand { level: AtomicU8, detail: AtomicU8 }     // written by the app after every layout solve
pub struct Cadence { pub hidden: Option<Duration>, pub visible: Duration, pub focused: Duration, pub always_on: bool }
pub struct SourceInfo { pub id: SourceId, pub produces: &'static [&'static str], pub cadence: Cadence, pub requires: &'static [Capability] }
pub struct SourceCtx { pub id: SourceId, ch: Channels, pub clock: Clock, pub stop: Arc<AtomicBool>, pub demand: Arc<Demand>,
                       pub ctl: Receiver<Control>, pub options: toml::Table }
impl SourceCtx {
    pub fn emit(&self, at: Ts, samples: Vec<Sample>);        // data.try_send; a drop increments a counter carried by the next status
    pub fn status(&self, s: SourceStatus);                   // control.send — never dropped
    pub fn alert(&self, e: AlertEvent);                      // control.send — never dropped
    pub fn sleep_until(&self, deadline: Ts) -> bool;         // zero-poll: parks on the ctl receiver (recv_timeout) until the deadline or Control::Stop; false when stopped.
    pub fn inject(&self, msg: Msg);                          // replay only (D48): re-emit a journaled message as the source it came from — batches on `data` *blocking*, controls on `control`, inputs on `input`
                                                             // Pollers compute deadlines as the next multiple of their cadence on the shared [perf] phase_ms grid, so one wake-up serves several sources
}
pub enum Control { Stop, SetOption(String, toml::Value), Restart, Domain(Box<dyn Any + Send>) }
pub trait Source: Send + 'static { fn info(&self) -> SourceInfo; fn run(self: Box<Self>, cx: SourceCtx); }
pub trait AsyncSource: Send + 'static { fn info(&self) -> SourceInfo; fn run(self: Box<Self>, cx: SourceCtx) -> BoxFuture<'static, ()>; }
pub struct SourceDef { pub info: SourceInfo, pub start: fn(&toml::Table) -> Box<dyn Source>, pub demo: fn(u64) -> Box<dyn Source> }
pub trait Sampler: Send + 'static { fn sample(&mut self, now: Ts, detail: u8) -> Result<Vec<Sample>, SourceError>; }   // poll helper: cadence, backoff 250 ms→30 s
```

Sources are **singletons per kind**, configured only under `[sources.<id>]` in `config.toml`; a `Command::Source(audio, Domain(SetSink))` therefore changes the sink for every audio tile. `Level::Paused` (htop's `Z`) makes sources emit nothing except `always_on` ones (pins). `detail` is the max of `Component::demand(tier)` over the visible instances that list the source (own or optional): `Meters` costs nothing extra, `Table` turns on the pid-level process scan, `Columns` is reached only by a `full` tier (zoomed or `view = "full"`) and is refined per column — the component sends `Command::Source(cpu, Control::SetOption("columns", …))` when its column set changes, so the cpu source reads `io`/`smaps_rollup`/`cgroup`/… only for enabled columns and walks `task/` only when userland threads are shown, exactly like htop's `PROCESS_FLAG_*`. `FocusLost` drops every source to `Hidden` and `Meters` (pins stays `always_on`). Replay is one `JournalSource`; demo wraps `store::demo::Synth` per source.

### 4.4 Alerts

```rust
pub enum Severity { Info, Warn, Crit }
pub struct AlertId(pub Arc<str>);                                 // "pins/overload", "rule/gpu-hot"
pub enum Transition { Raised, Repeated, Resolved }
pub struct AlertEvent { pub id: AlertId, pub source: SourceId, pub severity: Severity, pub transition: Transition, pub title: Arc<str>, pub detail: Arc<str>, pub at: Ts }
pub struct AlertLog { active: BTreeMap<AlertId, Active>, ring: Ring<AlertEvent> /* 500 */ }
pub struct RuleEngine { by_name: HashMap<&'static str, Vec<Rule>>, pending: HashMap<(AlertId, Label), Ts> }
```

Domain alerts arrive as `ControlMsg::Alert` (the pins source runs astral-watch's `Lifecycle`, tracks the active `Condition` set itself and emits transitions); `[[rules]]` from `config.toml` run inside `apply` on touched metrics via a name-indexed map (NaN/absent never raise or clear). The overlay keys on `AlertId` transitions, never on samples: a one-row banner under the tab bar on every page while a `Crit` alert is active and unacknowledged (pulsing reversed/plain on the heartbeat — no `SLOW_BLINK`; it yields when the body could not hold one tile), `a` acknowledges every active id until the next `Raised`, Warn-only states get a key-bar chip, `Resolved` a green toast, `A` opens the `alerts` view as an overlay (arc 3a, D50 §7, D51). `Command::Source(id, ctl)` and `Command::Ack(id)` route through `Shell::new`'s `controls` (per-source `Control` senders) and the acked set (D50 §6).

### 4.5 Journal

JSON Lines: a header (`version`, `wall_epoch`, `host`, `sources`, terminal size) then one line per `Batch`, `Status`, `Alert` (input only with `--record-input`). Concretely (D47): the header is `{"v":1,"wall_epoch","host","size":[w,h],"sources":[…]}`; every other line is `{"t":<Ts ns>, "b"|"st"|"al"|"in": …}` where a batch is `{"src","s":[["name{label}", datum], …]}` and the datum's JSON type selects its kind — number → Scalar, array → Vector, object → Record via `decode`. Replay is a `Source` (`JournalSource`) that drives `Clock::Virtual` to each line's `t` at `--speed N` and re-emits through the normal channels; the recorder is a bounded-channel tee from the frame loop to a writer thread that may drop (counted in the HUD) but never stalls a frame. Scalars/vectors are plain JSON; records use `RecordValue::to_json` on write and `KeyMeta::decode` on read; names are interned through `lookup`. `Replay` drives the virtual clock; `apply_all`/`apply_until` feed tests without timing. `--tables off` omits `proc.table` and `gpu.procs`. Size: a cpu-only journal is ≈ 11 KB/s (≈ 6.8 MB per 10 minutes) while the Overview's cpu tile is *focused* — 32 cores × three keys re-published every 500 ms is 92 % of it — and about a third of that at the 1.5 s visible cadence; tables would add a ≈ 60 KB `proc.table` per scan on top (measured 2026-09-01, arc 2a). A round-trip unit test iterates every `KeyMeta` with `kind == Record` and asserts `decode(to_json(x)) == x`.

### 4.6 Component contract (`gridwatch-ui`)

```rust
pub struct Footprint { pub w: u8, pub h: u8 }                       // TILE 1x1, WIDE 2x1, PANEL 4x2, HERO 6x3 — picker hints only
pub struct Tier { pub name: &'static str, pub min: Size, pub adds: &'static [&'static str], pub zoom_only: bool }   // cumulative: tier i draws tier i−1 plus `adds`; zoom_only tiers never appear by size alone
pub enum Chrome { Themed, Borderless, Custom }
pub enum Capability { Procfs, Hwmon, Cpufreq, Rapl, Nvml, I2cNvidia, AstralExporter, AstralCsv, PwRecord, PipeWireSocket,
                      DbusSession, PingSocket, NetRaw, TrueColor, VteGlyphs, Mouse }
pub struct Manifest {
    pub kind: &'static str, pub name: &'static str, pub summary: &'static str, pub contract: u32,
    pub footprints: &'static [Footprint], pub default_footprint: Footprint,
    pub requires: &'static [Capability], pub optional: &'static [Capability],
    pub sources: &'static [SourceId], pub optional_sources: &'static [SourceId],   // both contribute to Demand; absence of an optional one degrades a tier
    pub chrome: Chrome, pub keys: &'static [KeyHint], pub example_options: &'static str,
}
pub struct ComponentDef { pub manifest: &'static Manifest, pub build: fn(&mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> }
pub struct Registry { components: BTreeMap<&'static str, ComponentDef>, sources: BTreeMap<&'static str, SourceDef> }

pub struct RenderCx<'a> { pub inner: Rect, pub tier: usize, pub view_fallback: bool, pub focused: bool, pub captured: bool, pub zoomed: bool,
                          pub dense: bool, pub store: &'a Store, pub theme: &'a Theme, pub now: Ts, pub wall: SystemTime, pub tz_offset_s: i32, pub frame: u64 }
// `tz_offset_s`: local-time offset computed once by the app (libc `localtime_r`, the one unsafe seam) so components render local wall time
// deterministically — testkit passes 0. Under a virtual clock, `wall` is driven by the clock, so `shot --seed N` is byte-deterministic (D41).
pub struct TickCx<'a>   { pub store: &'a Store, pub now: Ts, pub visible: bool, pub tier: usize }
pub struct InputCx<'a>  { pub store: &'a Store, pub inner: Rect, pub caps: &'a CapSet, pub readonly: bool }
/// The semantic view tree (D32). Components describe *what* is shown; the theme's `Renderer` decides *how*. Small on purpose.
pub struct Span { pub role: Role, pub text: Cow<'static, str>, pub bold: bool }   // never a Color
pub enum View {
    Text(Vec<Vec<Span>>),                                                            // lines of spans
    KeyValue(Vec<(Cow<'static, str>, Vec<Span>, Option<Severity>)>),
    Gauge { label: Cow<'static, str>, value: f32, gradient: GradientId, text: Option<Cow<'static, str>> },
    Segmented { label: Cow<'static, str>, segments: Vec<(Role, f32)>, text: Option<Cow<'static, str>> },   // one horizontal multi-segment meter (htop's CPU/mem/swap bars); fractions sum ≤ 1 (D37)
    Bars { values: Vec<f32>, gradient: GradientId, labels: Option<Vec<Cow<'static, str>>>, peaks: Option<Vec<f32>> },
    Sparkline { series: Vec<Option<f32>>, gradient: GradientId, max: Option<f32> },
    Chart { series: Vec<Series>, bounds: Bounds, marker: MarkerHint },
    Table { columns: Vec<Column>, rows: Vec<Vec<Vec<Span>>>, selected: Option<usize>, sort: Option<(usize, SortDir)>, scroll: usize },
    BigNumber { text: Cow<'static, str>, role: Role },
    Stack { dir: Dir, children: Vec<(Constraint, View)> },
    Custom { paint: Box<dyn Paint>, describe: Cow<'static, str> },                   // bespoke surfaces (pins limit line, Winamp skin, scope); styles only via theme roles
}
pub trait Paint { fn paint(&self, area: Rect, theme: &Theme, buf: &mut Buffer); }
pub trait Renderer { fn render(&self, view: &View, area: Rect, theme: &Theme, buf: &mut Buffer); }   // the default lives in gridwatch-ui; a theme picks widget variants via [widgets]

pub enum Redraw { No, Yes }
pub enum RedrawPolicy { OnChange, Animated { fps: u8 } }
pub enum Outcome { Ignored, Consumed, Command(Command), Release }

pub trait Component: Send {
    fn manifest(&self) -> &'static Manifest;
    fn title(&self, max_width: u16, cx: &TickCx) -> Cow<'_, str>;
    fn tiers(&self) -> &'static [Tier];                               // poorest first; tiers[0].min must fit the grid's min_unit_inner; zoom_only tiers form a suffix
    fn demand(&self, tier: usize) -> Detail { Detail::Meters }         // what this tier needs from every source in sources ∪ optional_sources; the app writes the max over visible instances
    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw;                     // derive per-generation state, advance animations; no I/O
    fn view(&self, cx: &RenderCx<'_>) -> View;                        // pure over store + now; returns the semantic tree the theme's renderer draws
    fn redraw_policy(&self) -> RedrawPolicy { RedrawPolicy::OnChange }
    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome { Outcome::Ignored }        // store::InputEvent types
    fn on_mouse(&mut self, ev: MouseEvent, local: Position, cx: &InputCx<'_>) -> Outcome { Outcome::Ignored }
    fn on_visibility(&mut self, visible: bool) {}
}
pub enum Command { Quit, Page(usize), Zoom, Ack(AlertId), Toast(Severity, String), Record(bool), SaveLayout,
                   Source(SourceId, Control), Run(ActionId, Box<dyn Action>) }          // ActionId makes "keys in, commands out" tests addressable (D42)
pub trait Action: Any + Debug + Send { fn run(self: Box<Self>) -> Result<String, String>; }   // gains &ExecCx with the executor thread (arc 8, D42)
```

**Tier selection.** Tiers are cumulative supersets; the shell picks the richest tier whose `min` fits the inner rect, skipping `zoom_only` tiers unless the component is zoomed or the placement names that tier as its `view`. Tool-parity `full` tiers (htop's screens and F-key bar, nvtop's sortable process table with the signal menu) are `zoom_only`: on the grid a 6x3 tile shows the tool's *dashboard* face — meters, cores and a top-N table — and `z` gives the whole tool. A placement's `view = "table"` names a *preferred* tier: it is used when its `min` fits, otherwise the richest fitting tier is used and `view_fallback` is set so the title shows a `view↓` chip; an unknown `view` name is a config warning and is ignored. Zoom gives the component the whole body, hence its richest tier. There is no `SizeClass`, `Shape` or `footprint` in the render context: the inner rect and the tier index are the whole truth. `tick(&mut self)` hosts animation and derived state (Winamp peak-fall, htop tomb rows, the process table sorted once per source generation); `view(&self)` is byte-deterministic, and so is the default renderer for a given theme. `Command::Run(Box<dyn Action>)` keeps the command set open; actions are `Debug`-printable for "keys in, commands out" tests and executed on the executor thread.

### 4.7 Plugin contracts and the wire protocol (D32)

A *module* is a plugin against two contracts: `Source` (data in) and `Component` (view out), described by a `Manifest` and found through the `Registry`. In-process plugins are static — Cargo features, `builtin_registry()` — because Rust has no stable ABI. The **public** plugin API is the wire protocol the `exec` host speaks (arc 8), and later WASM on the same schema:

```
host → plugin   hello   { contract: 1, capabilities: [...], keys: [...] }            once
plugin → host   manifest{ kind, name, footprints, requires, sources, produces, keys }  once; validated against schema/manifest.schema.json
plugin → host   sample  { key, label?, at, value }                                    any time (a source plugin)
host → plugin   render  { instance, tier, inner: {w,h}, now, focused }                when a visible tile's inputs changed
plugin → host   view    { instance, tree }                                            the View as JSON (schema/view.schema.json); no cells, no colours
host → plugin   key     { instance, key }  ·  plugin → host   command { ... }         captured keys and Command values
plugin → host   status  { state, reason, hint }                                       health, like SourceStatus
```

JSON lines over stdin/stdout, one subprocess per plugin instance kind, supervised like a source (backoff, restart counter, `Unavailable` chip). A plugin can be a source, a component, or both. Because a plugin returns a *view tree* and not cells, it cannot break the theme, the layout or the readability rules, and the host renders it with the same `Renderer` as built-ins. Every format that crosses a boundary has a JSON Schema in `schema/` — `config`, `layout`, `theme`, `journal`, `view`, `manifest`, `exec` — with fixtures validated in CI; the `contract` number is the compatibility promise.

## 5. Data flow, threading, tick rates

```
input ─ event::read() → InputEvent ─────────────────────── input (unbounded) ─┐
src-cpu/gpu/pins/net/audio-io/audio-dsp/sensors  ─ Batch ── data (4096, lossy) ─┤  render (main): Store, App, Terminal,
src-async (tokio current_thread; mpris, probes)  ─ Status/Alert ── control ────┤  effects (!Send)
exec (Actions) ─ Done ── control ──────────────────────────────────────────────┤     │ Demand atomics
watch (1 Hz mtime stat of config/layout/theme) ─ Reload ── control ────────────┘     ▼ sources
```

Frame loop: drain input, then control, then ≤3 ms of data (`apply` each, tee to the recorder); re-solve layout only when page/size/zoom/dense changed (cached); write `Demand` per source (level from the visible cells whose `sources ∪ optional_sources` include it; `detail` from the richest visible tier); run `tick` for visible components; for each dirty visible instance call `view` and hand the tree to `theme.renderer()` (the orchestration is the shell's: tier → view → render → cache → ambient); draw only if dirty: a needed source's generation advanced, an animated visible component is due, effects are running, or the 1 Hz heartbeat fired. Frame cadence is `fps` (30) raised to `fps_max` (60) only while a visible component reports `Animated`. Pages not shown never cause redraws. **Render cache:** each instance's inner cells are kept with the key they were rendered under — `(generations of its sources, tier, inner rect, theme id, zoomed, focused, animation frame)` plus the hash of its last `View`, so a component whose data changed but whose tree did not (a stable value) is not even re-rendered — and a frame re-renders only instances whose key changed, blitting the rest with `Buffer::merge`; an animated tile therefore costs its own rect plus one whole-frame diff. **Coalescing:** frames are drawn on the frame clock, so several sources advancing inside one 33 ms slot yield one frame, and phase-aligned cadences make that the common case — the Overview with silent audio draws ≈ 2 frames/s (gpu and pins at 500 ms coincide; cpu, net and sensors land on the same 1 s boundaries). **Ambient layer:** a showcase-class theme's ambience (§7) runs after tiles, chrome and overlays at its own `fps`: it reads the finished frame from the render cache, keeps a per-cell brightness, lets the falling rain re-light the cells it passes and fades the rest, leaves pinned tiles fully lit, and is frozen on `FocusLost`/pause; it never invalidates the render cache.

| Source | Cadence (hidden / visible / focused) | Notes |
|---|---|---|
| cpu (procfs) | meters 3 s / 1.5 s / 500 ms; process scan (`Detail::Table`) 3 s / 3 s / 1.5 s, pid-level only (`stat`, `statm`, dir `st_uid`, `cmdline` on first sight) ≈ 10–15 ms on torch; `task/` walk and htop's gated files only at `Columns` | also reads k10temp `Tccd*` by label until the sensors source exists (key name unchanged); htop itself refreshes at 1.5 s and costs ≈ 3 % of a core doing so |
| gpu (NVML) | fast tier 1 s / 500 ms / 250 ms (≈ 20 µs under load; ≈ 1.6 ms per tick when the card idles in P8 — D49); slow tier 1 s (memory, enc/dec, PCIe counters); fans %/RPM every 5 s; `samples(Power)` while a gpu tile is visible (D49); process rows only at `Detail::Table`, on a 2 s grid (D49); static once; one `Device` handle per generation | never `pcie_throughput` (21 ms); byte-counter fields 197/198; keys labelled `{0}`; nvtop refreshes at 1 s — measured on torch (arc 2b, idle, release, 30 s): fast 0.04 + slow 2.55 + procs 1.67 = **4.26 ms/s** with process rows |
| pins (astral-watch) | 1 s / 500 ms / 500 ms, `always_on`; **5 s while the chip answers implausibly** (a deeply idle GPU makes astral-watch re-probe bytewise, 36 transactions — P14) | never paused; alerts depend on it; one `Lifecycle` per run, backends come and go underneath it |
| net | 2 s / 1 s / 250 ms + EWMA; link attrs 5 s; conns 2 s at `Detail::Table`; probes 1 Hz | collects every interface; instances filter |
| audio | pw-record killed after 10 s hidden; DSP at `[sources.audio] fps` (30 default, 60 opt-in) while visible; 2 Hz while input is below the floor | 1024-frame chunks (~10.7 ms) → rtrb → DSP thread |
| mpris | event-driven; Position poll 1 Hz while Playing | private tokio runtime, `select!` over streams + commands |
| sensors | 5 s / 1 s / 1 s | hwmon keyed by name@devpath |

Startup: NVML init, `detect_bus`, pw-record spawn and D-Bus connect happen on their source threads; the capability probe is time-boxed (≤200 ms, cheap checks only — file/socket/env existence and `libnvidia-ml.so.1` on the loader path; NVML initialises once, in its source) so the first frame with placeholder tiles appears in <300 ms.

## 6. Layout engine

Fixed unit grid per page, **12 columns × 6 rows** by default (24 columns allowed per page for hero layouts), placements `{ id | kind, at, size, view, priority }` in fixed units, solved by a pure integer track function — no solver, deterministic, invertible for the mouse:

```rust
pub struct GridSpec { pub columns: u8, pub rows: u8, pub gap: u8, pub borders: BorderMode /* Each | Shared | None */, pub cell_aspect: f32,
                      pub min_unit_inner: Size /* 8×3 */ }
pub enum SolveMode { Configured, Dense, Stack }
pub fn tracks(len: u16, n: u8, gap: u16) -> Vec<(u16, u16)>;   // widths differ by ≤1, exact sum, monotonic
pub fn thresholds(spec: &GridSpec, chrome_rows: u16) -> (Size /* configured */, Size /* dense */);
pub fn solve(spec: &GridSpec, page: &Page, body: Rect, mode: SolveMode, zoom: Option<&InstanceId>, tiers: &dyn Fn(&InstanceId) -> &'static [Tier]) -> Solved;
pub fn hit(s: &Solved, pos: Position) -> Option<&Cell>;  pub fn unit_tracks(spec, body, mode) -> (Tracks, Tracks);  pub fn unit_at(spec, body, mode, x, y) -> Option<(u8, u8)>;  pub fn unit_rect(spec, body, mode, at, size) -> Option<Rect>;  pub fn footprint_cycle(&[(u8,u8)], current) -> Option<(u8,u8)>;  pub fn focus_dir(…) -> Option<InstanceId>;   // unit_at ∘ solve is proptested (arc 4a)
pub fn move_by / resize_by / swap / insert_first_fit / remove(...) -> Result<Page, EditError>;   // pure, proptested
```

Rows are always explicit (placements are in fixed row units, so an "auto" row count would invalidate them; the research's auto-rows formula is dropped). Instead the **mode is derived from the terminal size**: `configured` needs `columns × (min_unit_inner.w + 2) + (columns − 1) × gap` columns and `rows × (min_unit_inner.h + 2) + (rows − 1) × gap + chrome` rows (131 × 37 for the defaults); `dense` (gap 0, shared borders via one-cell overlap + `Block::merge_borders(MergeStrategy::Exact)`, short titles, tab bar hidden) needs `columns × (min_unit_inner.w + 1) + 1` × `rows × (min_unit_inner.h + 1) + 1 + chrome` (109 × 27); below that the page becomes a priority-ordered vertical **stack** with scrolling. Transitions back to a richer mode require the terminal to exceed the threshold by 2 cells in both dimensions (hysteresis), and no single starved cell ever changes the page mode. Measured: 250×70 configured → inner 1x1 = 17×8, 2x1 = 38×8, 4x2 = 80×20, 6x3 = 122×31; 120×40 dense → 1x1 = 9×5, 2x1 = 19×5, 4x2 = 39×11, 6x3 = 59×18. Because every placement is ≥1 unit and every component's `tiers[0].min ≤ min_unit_inner` (enforced by `assert_min_tier_fits`), a starved chip (`▪ gpu`) remains only as a defensive fallback. Rounded-corner themes are forced to `BorderMode::Each` because `Exact` cannot merge rounded glyphs. A placement of a kind that is not compiled in or not yet registered renders a placeholder chip with the reason.

## 7. Theme system

Components ask for `Role`, `GradientId`, glyph names and severity; they never see a colour literal.

```rust
#[repr(u8)] pub enum Role { Bg, Surface, Panel, Border, BorderFocused, Title, Text, TextMuted, TextGhost, AccentPrimary, AccentSecondary, AccentTertiary,
                            Ok, Warn, Crit, Info, SelectionFg, SelectionBg, Cursor }          // arrays, no enum-map (3.x needs Rust 1.95)
pub enum GradientId { Load, Temp, Power, Mem, NetRx, NetTx, Audio, Title }
pub enum ColorMode { TrueColor, Ansi256, Ansi16, Mono }
pub struct Gradient { lut: [Color; 64] }   // Oklab-interpolated via palette 0.7.7, pre-downsampled per ColorMode
pub enum PerfClass { Quiet, Showcase }                                  // Showcase themes may spend CPU/terminal budget on ambience while focused (S-ceilings in PERFORMANCE.md)
pub struct WidgetSet { pub gauge: GaugeStyle /* Bar | Line | Block */, pub bars: BarStyle /* Eighths | Shade | Dots */, pub sparkline: SparkStyle /* Eighths | Braille */, pub table_header: HeaderStyle /* Underline | Reverse | Plain */, pub big_number: PixelStyle }
pub struct Theme { colors: [Color; 19], gradients: [Gradient; 8], pub glyphs: GlyphSet, pub borders: Borders, pub title: TitleSpec, pub widgets: WidgetSet,
                   pub flourish: Flourish, pub effects: EffectHooks /* data only */, pub ambient: Option<AmbientSpec>, pub class: PerfClass, pub mode: ColorMode }
impl Theme {
    // free functions in `theme`: build_theme(file, parent: Option<&ThemeFile>, mode) — parent = the resolved `inherits`, one level (D52); builtin_file(name) — a built-in, flattened; merge(child, parent)
    pub fn color(&self, r: Role) -> Color; pub fn style(&self, r: Role) -> Style; pub fn gradient(&self, g: GradientId) -> &Gradient;
    pub fn severity(&self, s: Severity) -> (Style, &str);                       // colour + glyph; Crit adds BOLD|REVERSED; meaningful in Mono
    pub fn block<'a>(&self, title: Line<'a>, chrome: Chrome, focused: bool, dense: bool) -> Option<Block<'a>>;   // None for Borderless/Custom
    pub fn for_kind(&self, kind: &str) -> &Theme;                               // the `[components.<kind>]`-derived theme, built once at load (D52); the shell passes it in RenderCx
    pub fn contrast_report(&self) -> Vec<String>;                               // the WCAG pairs with their ratios (`config check --theme`)
    pub fn renderer(&self) -> &dyn Renderer;                                     // the default renderer parameterised by `widgets`; themes choose form, components choose content
}
```

Colour mode resolves once: CLI `--color` > config > `NO_COLOR` (loads the real `mono` theme, because crossterm 0.29 emits no colour SGR at all) > `COLORTERM=truecolor` > `TERM` 256. The 256/16 nearest-colour mapper is in-tree (~25 lines; `ansi_colours` is LGPL). Glyph tiers: `ascii`, `unicode` (U+2500–259F, U+25xx shapes, braille), `nerd` opt-in only; `chart_marker = "octant_if_vte"` uses octants (VTE draws them natively) with braille fallback. `Role::Bg` is painted over the whole frame first every draw. The loader is staged: arc 1 shipped roles, `$palette`, gradients, glyph tiers, borders, title styles and the colour ladder with every file **self-contained** (D37); arc 3b's loader v2 (D52) adds `inherits` **one level** (the child overrides its parent key by key, the merged result must be complete — the schema requires every role and gradient only when there is no `inherits`), `[components.<kind>] gradients.<id>` overrides (a derived theme per kind, `Theme::for_kind`), colour values `default` | `#rrggbb` | the sixteen terminal names (`red`, `bright-red`, …) | `ansi:N` (a gradient with a non-RGB stop steps instead of interpolating), and the **WCAG warn gate**: `text` on `panel`/`surface` below 4.5:1 or `text_muted` below 3:1 warns at load (toasted at start, printed with the full report by `config check --theme`); `text_ghost` is the decorative role (empty-bar fill, gauge track) and has no floor. `[flourish]` and `[effects]` are consumed from arc 4b (D54 seams 6–7: `EffectHooks` data in `gridwatch-ui`, the tachyonfx painter in `gridwatch-app::effects` with `budget_ms`'s watchdog and `--no-effects`; `Flourish` drawn by the shell in the empty units; `[contrast] autofix`; `[ambient]` → `AmbientSpec` for `gridwatch-app::ambient`, with the ninth `rain` gradient and `[glyphs] rain`), when tachyonfx hooks (`startup`, `theme_swap`, `focus`, `alert`, `ambient`) are mapped in `gridwatch-app`, area-scoped, ≤600 ms for event effects, ambient CRT off by default, with a `budget_ms` watchdog. Built-ins: `modern` (Catppuccin Mocha), `retrowave` (Synthwave '84 + #ff2975 with the computed contrast fixes), `mono`, then `terminal`, `phosphor-green`, `phosphor-amber`, and the showcase-class `matrix`.

**Showcase themes and the ambient layer (D28).** A theme declares `class = "quiet"` (default — every ceiling in `PERFORMANCE.md` applies) or `class = "showcase"`: it may run an **ambient effect** over the whole frame at its own `fps`, and while such a theme is active *and the terminal is focused* the S-ceilings apply instead of P2/P6–P10; on `FocusLost` or `space` the ambient layer **freezes** (no frames), so P4 holds unchanged — a showcase theme never costs anything while you are in the game. The ambient layer is a post-render pass in `gridwatch-app` (`AmbientSpec` is data in `gridwatch-ui`; the painter is a tachyonfx `effect_fn` with a `CellFilter` mask): components are not re-rendered for it (the render cache holds), it writes into the frame buffer after tiles and chrome, and its changed cells go through the normal diff. The first ambient kind is `matrix_rain`: seeded column droplets (head glyph bright, trail through the `Rain` gradient, glyph mutation in place) drawn from the `rain` glyph set — half-width katakana U+FF66–U+FF9D, digits and a few symbols, all one cell wide (East Asian Width `H`; on torch fontconfig resolves them to Noto Sans Mono CJK JP), with `rain = "ascii"` as the fallback set. **Only the rain draws (D31, composed per D34).** Under `matrix` the ambient layer is the **sole compositor**: widgets, chrome and titles never reach the screen directly. The finished tile — its real glyphs and colours from the render cache — is a *mold*, and the falling rain is the only thing that puts light on screen. Every content cell carries a brightness `lit ∈ [0, 1]`. Rain falls continuously over the whole frame (density `rain.density`, speed `rain.speed`); when a droplet's head passes a content cell, that frame shows the droplet's rain glyph at head-white and the next frame shows **the module's own character** at `lit = 1` — the rain becomes the character — after which `lit` decays over `light.fade_s` (12 s): the cell's colour runs from head-white through its own theme colour (the bar's gradient stop, the text role) down to `light.floor` — **black by default (D34)** — so the module fades out entirely until the next characters fall. A module is therefore never *drawn* under `matrix`; it is *printed* by the rain, cell by cell, and forgotten at the rate the rain stops touching it — there is no widget layer over the rain and no rain layer over the widgets, only one field. Two rhythms overlap: the steady sparse rain keeps random cells shimmering, and a **sweep** every `light.sweep_s` (20 s) sends a dense fall across every column so the whole page is re-rendered in one coherent pass and then fades together — that is the cycle: a bright, complete render decaying toward black until the next fall. Over gutters, empty cells and the space around big glyphs the rain is ordinary rain, and its katakana trail fades at the fast `trail_ms` (900) rate — while a printed content character fades at the slow `fade_s` rate. The two decays are the whole picture: the rain remembers the *shape* of whatever it fell through, longer where that shape meant something. Chrome — borders, titles, the tier label — is content like any other: printed by the rain, fading like the rest. **Refresh as light.** With `relight_on_update = true` a cell whose value changed since the previous generation is re-lit to 1 the moment it changes, so live numbers, moving bars and the visualizer stay bright by virtue of changing, while static values are the ones that fade — brightness reads as *both* "recently rained on" and "recently changed". Nothing else is phased or mutated: bars stay bars, digits stay digits, text stays text; the only thing the rain alters is how lit each real cell is. Some things are continuously printed at full brightness (conceptually: saturated by rain), whatever the theme says: the **focused tile**, any tile with an **active Warn/Crit alert** (a pins overload must be readable through the rain), the **alert banner and toasts**, the status/key bar, the tile under the mouse pointer, and anything the user just touched — `reveal = ["focus", "alert", "hover", "key"]` re-lights the affected tile for `reveal_ms` (2 500) after a key press, `V` re-lights the whole page (an instant sweep), and `L` locks every tile fully lit (rain in gutters only) until pressed again. **Governor.** The ambient layer measures itself: if the frame p95 exceeds 16 ms or the bytes/s exceed the S2 ceiling over a 2 s window it steps down — rain `fps` 24 → 16 → 12 → 8, then `density × 0.75`, then lengthens `sweep_s`, then gutters-only — and recovers one step per 30 s; the `F12` HUD shows the governor state. Cost shape: every frame changes the droplet cells (≈ density × 17 500) plus the cells whose fade crossed a colour step (the fade is quantised to the 64-entry `Rain` LUT, so a 12 s fade changes a cell only ~5 times per second at most, and not at all once it reaches ghost); a sweep changes most content cells for its duration. Determinism: the rain's RNG and sweep timing are seeded from the theme id and the frame index on the virtual clock, so replay and snapshots reproduce every frame byte for byte. State: a per-cell `lit_at: u32` frame index and the droplet pool — ≈ 4 bytes × 17 500 ≈ 70 KB for the whole frame.

## 8. Component catalogue

Every component: manifest, cumulative tiers (poorest first, `tiers[0].min ≤ 8×3`), data from the store, interactions once captured with `Enter`, degraded mode from `SourceStatus` and the capability set. Chrome is `Themed` unless stated.

**htop (`kind = "htop"`)** — source `cpu`: procfs 0.18 (`default-features = false`) with htop's formulas verbatim (guest subtracted, `cached = Cached + SReclaimable − Shmem`, Irix-mode CPU%, deltas keyed by `(pid, starttime)`), topology from sysfs `die_id`/SMT pairs, Tccd temps read by label from k10temp until the sensors source takes the key over. The process scan (`Detail::Table`) is **pid-level**: `stat`, `statm`, the directory's `st_uid`, and `cmdline` on first sight of a `(pid, starttime)` — **5.4 ms mean / 6.3 ms worst on torch for 635 pids** (measured 2026-09-01, release), on its own phase-aligned grid of 3 s visible / 1.5 s focused beneath the meters cadence. htop's gated files (`io`, `smaps_rollup`, `cgroup`, `oom_score`, `autogroup`, `exe`, `cwd`) and the `task/` walk that lists userland threads run only at `Detail::Columns`, i.e. a `full` tier with that column on or `hide_userland_threads = false`. Keys: `cpu.total_pct`, `cpu.core_pct{n}`, `cpu.breakdown` (Record; unlabelled = `/proc/stat`'s aggregate `cpu` line, `{n}` per core), `cpu.freq_mhz{n}`, `cpu.topology` (Record `CpuTopology { die_of, core_of, die_temp }` — the die/core map the `cores` tier groups by, D43), `mem.*`, `swap.*` (incl. `swap.cached_b`, D43), `psi.*`, `tasks.*` (threads from `/proc/loadavg`, no task walk; `tasks.kernel` is counted by the pid-level scan and published only while it runs, and the task line switches to htop's `procs, N thr, N kthr` wording exactly then — arc 2a), `sys.load*`, `sys.uptime_s`, `sys.pid_digits`, `sys.scan_ms` (wall ms of the last pid-level scan — P15's evidence, published only while the scan runs; D48), `sensor.temp_c{k10temp:Tccd1}`, `proc.table` (Record `ProcTable(Arc<[ProcRow]>)`, see §8.1). Tiers (rows they occupy in brackets): `tiny` [3] (CPU% + 1-row sparkline, min 8×3), `big-number` [4] (+ big digits, min 12×4), `meters` [6] (+ StackedBar nice/user/kernel/virt through roles, mem/swap bars, tasks/load/uptime, min 30×6), `cores` [12] (+ 2 CCD blocks × 8 cores with SMT pairs, MHz, Tccd, PSI row, min 56×12), `table` (+ the top-N process table of §8.1 — grid default `PID RES SHR S CPU% MEM% TIME+ Command`; min 56×18 = 12 rows above + header + 5 rows), `full` (**zoom-only**: + Main / I/O screen tabs, every column with horizontal scroll, tree view, search `/`, filter `\`, tags, follow, `K`/`H` thread toggles, F-key bar as Chrome::Custom; min 100×24). View options: `hide_kernel_threads` (default true = htop's default), `hide_userland_threads` (**default true — a deliberate deviation from htop's default false**, so the grid table is not ten rows of one game's threads and the scan stays pid-level; `H` in the zoomed `full` tier flips it and enables the `task/` walk), `sort` (default `cpu`, descending; identifiers `pid user pri nice virt res shr state cpu mem time command`), `tree`, `table_rows` (default 10, min 5), `columns` (grid default `PID RES SHR S CPU% MEM% TIME+ Command` — `USER`, `VIRT`, `PRI`, `NI` are off on the grid for space (D27) and available here; the `full` tier defaults to htop's whole Main screen), `command_min` (default 20 cells), `highlight_base_name` (default false, htop's default), `highlight_changes` (default false) with `highlight_changes_delay = 5s`. Keys are htop's verbatim; `F9`/`k`, `F7`/`F8`, `a`, `i` emit `Action`s (nix 0.31 `kill`/`sched_setaffinity`, libc `setpriority`/`ioprio_set`) behind `readonly` and a confirm line (arc 8). Degraded: root-owned rows show `N/A` for io/exe like htop; htop 3.4's GPU meter and `GPU%` column exist but read DRM fdinfo, which the proprietary driver does not expose (verified: no `drm-*` lines on any GPU client here) — htop prints `0.0`/`N/A`, gridwatch a dash.

**gpu (`kind = "gpu"`)** — source `gpu`: nvml-wrapper 0.12.1 on its thread; fast tier util/temp/power(`POWER_INSTANT`)/clocks/pstate/throttle at 500 ms visible (250 ms focused, ≈ 20 µs per tick), slow tier 1 s: `memory_info` v2 (reserved excluded), enc/dec, PCIe byte counters diffed; fans %/RPM every 5 s (setpoints; nvtop reads % only, never RPM); `samples(Power)` 20 ms trace only while a gpu tile is visible (D49: `Demand` cannot name a tier); static once (name, arch, cores, bus width, VBIOS, limits, the slowdown threshold via `temperature_threshold(Slowdown)` — the T.Limit fields 193/194/196 return *margins*, not thresholds, and are not polled) plus a hand-verified `const SPECS` table keyed by PCI id (0x2B85 …) cross-checked against NVML. Process rows — `running_graphics_processes()` + `running_compute_processes()` (both v3, ≈ 0.35 ms incl. the wrapper's count + fetch pairs) overlaid with `process_utilization_stats(last_seen)` (≈ 1.7 ms) — are fetched on the slow tier **only while a visible gpu tile's `demand(tier)` is `Table` or richer**; `last_seen` is wall-clock µs = the newest sample timestamp returned by the previous call (initially `wall_us − tick_us`), samples with `timestamp ≤ last_seen` or util > 100 are ignored and processes without a fresh sample read 0, as nvtop does; `NotFound` ⇒ nothing fresh ⇒ zeros; `InsufficientSize` from the lists is transient (keep the previous rows, retry next tick) — only `NotSupported` is pruned. Slow tier ≈ 2.5 ms/s on torch, ≈ 4.3 ms/s with process rows on their 2 s grid (D49). Published as `gpu.procs{dev}` (Record `GpuProcs { rows: Vec<GpuProcRow>, vram_total_b }`, see §8.1). Manifest: `sources = [gpu]`, `optional_sources = [cpu]` — USER / CPU / HOST MEM / Command join `proc.table`; the `procs` and `full` tiers return `Detail::Table` from `demand`, which raises the cpu source's detail even when no htop tile is visible (the zoomed gpu tile is the guaranteed case); the four columns render `—` when the cpu source is absent. Keys (all `{dev}`): `gpu.util_pct`, `gpu.memctl_pct` (distinct from) `gpu.vram_used_b`, `gpu.power_w`, `gpu.power_trace` (Vector), `gpu.temp_c`, `gpu.fan_pct{dev:i}`, `gpu.pcie_rx_bps`, `gpu.info`, `gpu.procs`, plus `gpu.nvml_ms{fast|slow|procs}` — NVML wall ms per second per call class, P11's evidence, the one key labelled by class rather than device (D49). The full list is `docs/KEYS.md`. Tiers (rows they occupy in brackets): `badge` [3] (util + temp, min 8×3), `gauges` [5] (+ GPU/VRAM/MEMCTL + clocks/W/°C/fan + throttle chip, min 24×5), `header` [8] (+ nvtop header parity: PCIe gen@width RX/TX, ENC/DEC auto-hide, fans, 50 Hz power sparkline, min 56×8), `charts` [8 + 4..8] (+ ten-minute `resample` charts in a band of 4–8 rows that grows with height — util, VRAM %, temperature, power %, clock % and gridwatch's own effective load `util × power / limit` (not an nvtop 3.2.0 metric, PARITY) — selectable series, `-r` reverse, the GPU-Z spec column when width allows; min 56×12), `procs` (+ the top-N GPU process table of §8.1 — grid default `PID DEV TYPE GPU GPU MEM CPU HOST MEM Command`; min 56×18 = header 8 + 4 chart rows + header row + 5 rows), `full` (**zoom-only**: + every row with scrolling, `F6` sort by any of nvtop's criteria, `F9` signal menu, the Power sub-panel showing the six pin bars under board power; min 100×24). View options: `sort` (default `gpu_mem`, descending; identifiers `pid user type gpu enc dec gpu_mem cpu host_mem command`), `table_rows` (default 10, min 5), `columns` (grid default `PID DEV TYPE GPU GPU MEM CPU HOST MEM Command` — `USER` is off on the grid (D27); add `USER`, `ENC`, `DEC` as needed, nvtop hides ENC/DEC by default too; the `full` tier defaults to nvtop's set with `USER`), `command_min` (default 12 cells), `series` (chart selection, keys `1`–`6`), `reverse` (default false, key `r`), `spec_column`, `power_panel` (default true, `full` tier only). Degraded: `LibloadingError` → nvidia-smi CSV tier at 1–2 s (no process rows); `LibRmVersionMismatch` → "driver/library mismatch — reboot", no retry; `NotSupported` fields never polled again — an absent value renders `—` (nvtop's `N/A` only in the GPU MEM cell); `GpuLost` → re-init with backoff.

**pins (`kind = "pins"`)** — source `pins`: astral-watch pinned git rev (`default-features = false`, never `tui`/`safety`); auto-select exporter 127.0.0.1:9942 → direct i2c (`detect_bus`, `read_reading` ≥500 ms, `redetect_card` after 10 misses) → CSV tail (parity arc) → Unavailable, re-probed every 10 s; `Lifecycle` from `astral_watch::config::load(None)` runs in the source in every mode, which maintains the active `Condition` set from `Event`s and emits control-channel alerts (Overload/Disconnected/Imbalance → Crit; ImbalanceAdvisory → Warn; TelemetryLost → Info). Keys: `pins.amps{1..6}`, `pins.volts{1..6}` (1-based, the connector's numbering), `pins.total_a/total_w/balance` (balance absent when undefined), `pins.read_ms` (P14 evidence), `pins.info` (mode, bus, pci, model, interval, the thresholds/policy in force), `pins.state` (telemetry health, the active set, the exporter's flags) — D50. Tiers: `watts-badge` (total W + balance badge + alert glyph, min 8×3), `mini-bars` (+ six eighth-block bars, 9.2 A `┄`, min 20×4), `bars` (+ peak caps, values, balance gauge, totals, min 40×8), `trend` (+ watts sparkline + log + alarm row, min 60×14), `full` (zoom-only, min 100×24: + tui.rs parity — device header from `gpu.info` and `gpu.pcie_gen/width` (never sysfs), braille trend chart, scrollable log; keys `p` freeze the display, `r` reset peaks, `+`/`−` interval via `Command::Source`; multi-card tabs in the parity arc). View option `history` (300). Constants reused verbatim (AMPS_CEILING 10, 7.82/9.2 A bands, balance WARN 1.33/ALARM 1.5). Degraded: `PermissionDenied` → "add yourself to the i2c group"; `NoTelemetry` → "waiting for telemetry (GPU idle?)" with `TelemetryLost` fed to the lifecycle; contention with a root logger → EIO becomes TelemetryLost, never corruption (kernel per-adapter lock).

**net (`kind = "net"`)** — source `net` collects every interface: `/proc/net/dev` + sysfs link attrs, `/proc/net/route`, resolved DNS over zbus (optional), conns from `/proc/net/{tcp,udp}*` joined with own `/proc/*/fd` at `Detail::Table`; `net-probe` (async, feature): surge-ping DGRAM ICMP → TCP-connect fallback, 60-sample ring (min/avg/max/mdev/RFC3550 jitter/loss); Wi-Fi via neli-wifi behind a feature with NM D-Bus fallback. Keys: `net.{rx,tx}_bps{iface}`, `net.{rx,tx}_drop{iface}`, `net.link{iface}`, `net.route`, `net.conns`, `net.rtt_ms{target}`. Instance (view) options: `interfaces = ["en*", "wl*", "wg*", "tun*"]`, `hide = ["veth*", "br-*", "docker*", "virbr*"]`, `rdns = false`; source options: `probes`, `public_ip = false`. Tiers: `rates` (↓/↑ for the default-route iface + link dot, min 8×3), `sparks` (+ rx/tx sparklines + speed/SSID, min 20×5), `table` (+ ifaces, drops/errs, mirrored braille chart, probe strip, min 48×10), `conns` (+ top connections; uid when PID unreadable, min 70×16), `full` (+ sortable conn table, iface detail pane, probe pane). Degraded: ICMP `EPERM` → "tcp" chip; per-process bandwidth shows a capability badge until the `gridwatch-netd` helper arc.

**audio (`kind = "audio"`)** — source `audio`: supervised `pw-record --format f32 --rate 48000 --channels 2 --raw --latency 1024 --target auto -P '{ stream.capture.sink = true, node.passive = true, node.name = "gridwatch audio" }' -` (1024 keeps the PipeWire graph at its default quantum and delivers the same ~10.7 ms chunk cadence as 512; `low_latency = true` opts into `stdbuf -o0` + `--latency 256` only while the widget is visible); io thread → rtrb SPSC → DSP thread: cava-style dual FFT (realfft N=8192 for bands below 250 Hz, N=2048 above, both Hann), 64 log bands 30 Hz–16 kHz per channel, dBFS floor −65, +4 dB/oct tilt, scope 512 samples, RMS/peak; publishes `audio.bands{0,1}` (Vector — display resolution, not spectral), `audio.scope`, `audio.rms_db{ch}`, `audio.sink` (Record from `pw-dump` every 2 s while visible) at `fps` while the input is above the floor and 2 Hz otherwise; child killed after 10 s Hidden. Supervisor: respawn only on EOF/exit (250 ms → 5 s backoff), never on "no data" (`node.passive` on an idle sink delivers nothing); >250 ms without data = silence, bars decay. Component: resamples bands to its bar count and applies ballistics in `tick` (`preset = "winamp"` falloff/16 + accelerating peaks, or `"cava"` gravity/integral/monstercat); `RedrawPolicy::Animated { fps }` whenever visible. Tiers: `vu` (stereo VU/peak pair, min 8×3), `mini` (+ 8–10 thin bars, min 16×4), `scope` (+ Canvas octant on VTE, braille elsewhere, min 30×6), `spectrum` (+ mirrored stereo ⌊(w+1)/3⌋ thick NINE_LEVELS bars, gradient `Audio` per row, `▔` peaks, sink name, min 40×8), `full` (+ scope + VU + LUFS via ebur128). Keys: `m` mode, `g` gravity, `[ ]` range, `s` sink picker (`Command::Source(audio, Domain(SetSink))`, global). Degraded: pw-record missing → "install pipewire-bin"; no socket → Unavailable.

**winamp (`kind = "winamp"`)** — source `mpris` (async): hand-rolled zbus 5 `Player`/root proxies (`Position` `emits_changed_signal = "false"`, `CanControl` const), discovery by `ListNames` + `NameOwnerChanged` with `arg0ns`, per-player tasks with `select!` over property streams, `Seeked`, owner-changed and a 1 Hz Position poll while Playing; supervisor picks Playing > most recent > alphabetical; commands via `Control::Domain(MediaCmd)`. Art: `file://`/`https://` (ureq, 5 s, 8 MB cap)/`data:` → `image` 0.25 decode on the source thread, downscaled ≤256 px, stored as `media.art` (Record RGB), painted by the in-tree ▀ halfblock widget. `optional_sources = [audio]`: the vis area draws a static skin when `audio.bands` is absent. Keys: `media.players`, `media.now` (title/artist/album/status/`pos_us`+`read_at`/`len_us: Option`/rate/caps), `media.history`. Track identity = hash(title|artist|album|url); `len = None` ⇒ stream mode with a local elapsed clock. Chrome::Custom. Tiers: `status` (▶/‖/■ glyphs, marquee, posbar, min 8×3), `shade` (+ time + 8-bar mini vis, min 24×3), `main` (+ tui-big-text Quadrant digits, marquee at 220 ms steps from `cx.now` with `  ***  `, kbps/kHz/stereo from `audio.sink`, 19-band vis, posbar, volume, transport, shuffle/repeat greyed when unsupported, min 40×10), `main+art` (min 60×12), `full` (+ EQ weighting the vis bands + playlist from metadata history). Keys: `x c v b z` transport, `←/→` seek 5 s, `+/-` volume, `p` cycle player, `r` raise. Degraded: no session bus → placeholder; no players → idle skin.

**sensors (`kind = "sensors"`)** — source `sensors`: hwmon walker keyed by `name@devpath`, labels from `*_label`, `_max/_crit` with the nvme `65261850` sentinel dropped, chips with no inputs skipped; PSI; cpufreq per CCD; `amd_x3d_mode`; RAPL only if `energy_uj` is readable; takes over `sensor.temp_c{k10temp:*}` from the cpu source. GPU temp/fan/power appear in the same list from `gpu.*` keys, never polled twice. Keys: `sensor.temp_c{chip:label}`, `sensor.meta{..}`, `rapl.pkg_w`. Tiers: `hottest` (worst-margin reading + trend arrow, min 8×3), `strip` (+ chips as chips, min 24×3), `table` (+ grouped by chip with warn/crit bars + mini sparklines, min 40×8), `chart` (+ braille chart of selected sensors, min 60×14), `full`. Degraded: RAPL 0400 → "needs udev rule" chip.

### 8.1 Process tables (htop `table` / `full`, gpu `procs` / `full`)

The two tables are one mechanism with two column sets. Both read from records the render thread already holds; neither scans `/proc` itself. On the grid both use a **reduced default column set** (D27): the identity, priority and virtual-size columns that a dashboard glance never needs are off by default and live in the `columns` option and the zoomed `full` tiers.

**Data.** The cpu source's pid-level scan (`Detail::Table`; `stat`, `statm`, dir `st_uid`, `cmdline` on first sight; ≈ 5–6 ms per pass on torch, 3 s on the grid / 1.5 s focused) produces `proc.table: ProcTable { rows: Arc<[ProcRow]>, pid_digits: u8 }` with `ProcRow { pid, ppid, tgid, uid, user: Arc<str>, state, pri, nice, nlwp, virt_kib, res_kib, shr_kib, cpu_pct, mem_pct, time_cs, starttime, kthread, cmdline: Arc<str>, comm }` — `user` is resolved in the source (uid → name cache), CPU% is Irix-mode `Δ(utime+stime)/period·100` keyed by `(pid, starttime)`, memory fields are KiB as htop prints them, and `pid_digits` = digits of `/proc/sys/kernel/pid_max` clamped to 5..19 (7 on torch), so demo, replay and snapshots share the width rule. Userland threads are rows only at `Detail::Columns` with `hide_userland_threads = false` (the `task/` walk costs another ≈ 30 ms on torch); the thread-group leader's `stat` already sums its threads, so CPU% and TIME+ are complete without it. The gpu source publishes `gpu.procs{dev}: GpuProcs { rows: Vec<GpuProcRow>, vram_total_b }` (the record travels as `Arc<dyn RecordValue>`, like `ProcTable`) with `GpuProcRow { pid, kind: Graphics | Compute | Both, vram_b: Option<u64>, sm_pct, mem_pct, enc_pct, dec_pct, fresh: bool }` — `Both` when a PID appears in both v3 lists (nvtop prints such a PID twice, once per list; gridwatch shows one merged row — recorded in `PARITY.md`). The gpu component joins the two by PID in `tick` once per source generation and keeps the last-known `cmdline` for a PID across ticks (nvtop's per-PID cache) — there is no NVML name fallback, `nvmlSystemGetProcessName` reads `/proc/<pid>/cmdline` itself — rendering `[pid]` muted if nothing was ever read. The htop component sorts and filters in `tick` once per generation. `view` never sorts.

**Row budget.** `rows = if zoomed { available } else { min(table_rows, available) }` with `available = inner.height − rows_above − 1`, where `rows_above` is what the lower tiers occupy (htop `cores` 12; gpu `header` 8 + a chart band of `clamp(inner.height − 8 − 1 − table_rows, 4, 8)` rows). Each table tier's `min` guarantees at least **5** rows; `table_rows` defaults to **10** so a 6x3 tile reads as a dashboard, not a wall; zoom fills the body from arc 2 (scrollbar, `PgUp`/`PgDn`), and the `full` tiers add htop's/nvtop's paging and horizontal scroll. Because the reduced column sets fit in 56 columns, both table tiers have `min = 56×18` — the same width as `cores`/`charts` — so the tables appear in any 6x3 whose inner height reaches 18: on torch (250×70, 6x3 = 122×31) htop shows 10 rows (18 available) and gpu 10 (14 available with an 8-row chart band); a 4x2 (80×20) shows 7 rows in both; at **120×40 dense (6x3 = 59×18, or 59×19 on the grid's top row) both show 5–6-row tables** — the laptop layout keeps the PID lists. The layout test asserts these tier names per placement.

**htop table** — htop 3.4.1's Main screen; widths are the printed cell *including* its trailing separator space. Grid default set in bold: **`PID RES SHR S CPU% MEM% TIME+ Command`** (the `full` tier defaults to htop's whole Main screen):

| col | grid default | cells | source | notes |
|---|---|---|---|---|
| `PID` | **on** | `pid_digits` + 1 (8 on torch) | `pid` | right-aligned, `%*d ` with digits of `pid_max` (min 5, max 19) |
| `USER` | off (`columns`) | 10 + 1 | `user` | left-aligned; arc 2a mutes root-owned rows — htop's own-uid rule needs the source to publish its uid (BACKLOG), and the elevated-capabilities colour needs `status` (arc 8's gated files) |
| `PRI` | off | 3 + 1 | `pri` | `RT` when ≤ −100 |
| `NI` | off | 3 + 1 | `nice` | < 0 `Crit`, > 0 `Ok`, 0 muted |
| `VIRT` | off | 5 + 1 | `virt_kib` | same formatting as RES |
| `RES` `SHR` | **on** | 5 + 1 each | `res_kib` `shr_kib` | `Row_printKBytes`: < 1000 → five digits plain; 1000–99 999 → five KiB digits with the leading thousands in the M role and no unit (`28248`); ≥ 100 000 → three significant digits + unit (`97.6M`, `9.76G`, ` 100M`, `1000M`) with roles cycling Text → M → G → Large; `N/A` muted when unreadable |
| `S` | **on** | 1 + 1 | `state` | `R`/`U`/`t` → `Ok`; `D`/`Z`/`T`/`X`/`B`/`P` → `Crit` (`P`, parked, is htop's `BLOCKED` and prints as `B`); `S`/`I`/`W`/`Q` → `TextMuted`; anything else → `Text` |
| `CPU%` | **on** | auto: 4 + 1, growing to `ceil(log10(max + 0.1)) + 2` over the rows of the *current* table — htop resets the width every scan cycle (`Row_resetFieldWidths`), so a burst widens the column while it lasts (6 on torch under a game) | `cpu_pct` | `< 0.05` muted, `≥ 99.9` accent; Irix mode, up to `active_cpus × 100` |
| `MEM%` | **on** | 4 + 1 | `mem_pct` | prints `100` above 99.9 |
| `TIME+` | **on** | 8 + 1 | `time_cs` | `MM:SS.hh`, `HHhMM:SS`, `DdHHhMMm`, `DDDDdHHh`, `YYYyDDDd` |
| `Command` | **on** | elastic | `cmdline` / `comm` | thread rows (userland or kernel, when shown) in the thread role, deleted executable `Crit`; `highlight_base_name` and `highlight_changes` (new rows `Ok`, vanished rows `Crit` for `highlight_changes_delay`) are off by default, as in htop |

The fixed part of the grid default is 41–43 cells (htop's full Main screen is 66–68); the sort column's separator carries htop's `▽`/`△` glyph. The `full` tier adds the I/O screen (`PID USER IO IO RATE DISK READ DISK WRITE SWPD% IOD% Command` — the `*D%` delay columns render `N/A` on torch exactly as htop does, because `task_delayacct` is off and Ubuntu's htop has no taskstats) and every other column htop lists through the `columns` option, scrolling horizontally like htop. On the grid, when `Command` would fall below `command_min` (20 cells), columns are dropped from the head of **gridwatch's** order — htop itself never drops, it scrolls — **SHR, TIME+, RES, PID, S, MEM%, CPU%** for the default set (with htop's full set enabled the order is `VIRT, SHR, PRI, NI, TIME+, USER, RES, PID, S, MEM%, CPU%`); `Command` is always last and elastic. At the tier's minimum width (56) the guaranteed set is therefore `PID RES S CPU% MEM% Command` (with a four-wide `CPU%` there is room for `TIME+` too; a six-wide one under a game drops it); at 80 and above the whole grid default fits with ≥ 37 cells of `Command`. Default sort `cpu` descending; `hide_kernel_threads` and `hide_userland_threads` on; selection, tags and collapsed subtrees are keyed by PID so a re-sort never moves the cursor.

**gpu table** — nvtop 3.2.0's process list; widths are nvtop's `sizeof_process_field` plus its one-space separator. Grid default set in bold: **`PID DEV TYPE GPU GPU MEM CPU HOST MEM Command`** (`DEV` hidden with a single GPU; the `full` tier defaults to nvtop's set with `USER`):

| col | grid default | cells | source | notes |
|---|---|---|---|---|
| `PID` | **on** | 7 + 1 | `pid` | |
| `USER` | off (`columns`) | max(4, longest name among the joined rows) + 1 | join `proc.table` | nvtop's rule: minimum 4, never truncated (8 on torch); gridwatch measures over every joined row, not only the page shown |
| `DEV` | **on** (auto-hidden with one GPU) | 3 + 1 | `{dev}` label | as nvtop |
| `TYPE` | **on** | 8 + 1 | `kind` | `Graphic` (`Warn` role — nvtop yellow), `Compute` (`AccentTertiary` — nvtop magenta), `Both G+C` (three-coloured) |
| `GPU` | **on** | 4 + 1 | `sm_pct` | header is `GPU`, values carry `%`; 0 when `fresh == false` |
| `ENC` `DEC` | off | 4 + 1 each | `enc_pct` `dec_pct` | nvtop's default field set omits them too; enable via `columns` |
| `GPU MEM` | **on** | 14 + 1 | `vram_b` | `12579MiB  38%` (`%6uMiB %3u%%`, used / total) |
| `CPU` | **on** | 6 + 1 | join `proc.table` | `100·Δ(utime+stime)/Δwall` — nvtop's formula, which is htop's CPU% |
| `HOST MEM` | **on** | 9 + 1 | join `proc.table` | RSS |
| `Command` | **on** | elastic | join `proc.table` | last-known value kept across ticks |

The grid default (single GPU) is 54 cells of fixed columns, so `Command` gets 26 at 80 and 68 at 122. Default sort `gpu_mem` descending; the `full` tier sorts by any of nvtop's criteria (`F6`, `+`/`−`) and opens nvtop's signal menu on `F9` through an `Action`. When `Command` would fall below `command_min` (12 cells) columns are dropped in the order **ENC, DEC, HOST MEM, TYPE, USER, CPU**; `PID`, `GPU`, `GPU MEM` and `Command` always survive — at the tier's minimum width (56) that means `HOST MEM` goes and `Command` keeps 12 (nvtop itself scrolls by 4 columns with `h`/`l`; the `full` tier does the same). Processes that only hold a context (`sm 0` — the terminal itself at 44 MiB) sort below active ones at equal memory.

**Shared behaviour.** Arc 2 (read-only tables): `↑/↓ PgUp/PgDn Home/End` select, `</>`/`F6` sort column, `I` invert, zoom fills the body. Arc 8: `/` search (htop), `F9`/`k` signal and the other `Action`s on the executor thread, tree/filter/tags, horizontal scroll. Both tables are snapshot-tested at the real 6x3 (122×31), 4x2 (80×20), dense 6x3 (59×18) and zoomed (248×66) sizes with `demo::Synth`'s 32-process synthetic set — a game at 12.5 GiB / 17 % SM, a shell, a browser, kernel threads, one `Both G+C` process (the terminal), and VIRT/RES values chosen to hit all three `Row_printKBytes` regimes — so the column drop order, the row budget and the formatting branches are pinned.

Free extras: `clock` (the 60-line template, tui-big-text, Chrome::Borderless — honoured by the shell from arc 1), `sources` (status / cadence / demand level and detail / age / dropped / restarts, plus the NVML ms/s and `/proc` scan ms the performance gates read — the debugging tile), `alerts` (scrollable log with ack).

## 9. Configuration

Two files plus a themes directory: `~/.config/gridwatch/config.toml` (behaviour, singleton sources, component instances, rules) and `layout.toml` (grid + pages + placements — the only file edit mode ever writes). Sources are configured **only** under `[sources.<id>]`; instance `options` are view-only (filters, presets, sort) and validated by each component's `Options` type; a test asserts that no option name appears both in a source's option struct and in the same domain's component `Options`. All via `toml 1.1` + serde with `deny_unknown_fields`, layered defaults ← file ← `GRIDWATCH_*` env ← CLI by hand; validation reports `file:line:col` via `toml::de::Error::span()`; overlaps name both ids; unsupported footprints warn only.

```toml
# config.toml
schema = 1
theme = "retrowave"           # modern | retrowave | mono | terminal | phosphor-green | phosphor-amber | matrix (showcase class) | <file>
fps = 30
fps_max = 60
color = "auto"                # auto | always | never | 16 | 256 | truecolor
mouse = true
readonly = false              # blanks kill/renice like `htop --readonly`
confirm_kill = true
[store]   history = "10m"   max_mb = 32
[record]  dir = "~/.local/share/gridwatch/recordings"   tables = false
[effects] enabled = true   budget_ms = 4
[perf]    unfocused_fps = 2   phase_ms = 250      # animated tiles drop to 2 fps when the terminal loses focus; pollers align to a 250 ms phase

[sources.cpu]   refresh_ms = 1500
[sources.gpu]   refresh_ms = 500   # the visible fast tier; device = 0
[sources.pins]  source = "auto"   exporter = "127.0.0.1:9942"   interval_ms = 500   # csv tail is arc 8 (D50 §5)
[sources.audio] sink = "auto"   latency = 1024   low_latency = false   fft = 2048   fft_bass = 8192   lo_hz = 30   hi_hz = 16000
                floor_db = -65   tilt_db_oct = 4   fps = 30
[sources.net]   probes = ["gateway", "1.1.1.1", "8.8.8.8"]   public_ip = false

[[components]] id = "cpu"    kind = "htop"    options = { hide_kernel_threads = true, hide_userland_threads = true, table_rows = 10 }
[[components]] id = "gpu"    kind = "gpu"     options = { power_panel = true }
[[components]] id = "pins"   kind = "pins"
[[components]] id = "lan"    kind = "net"     options = { interfaces = ["eno1", "wl*"], rdns = false }
[[components]] id = "viz"    kind = "audio"   options = { preset = "winamp", bars = "auto" }
[[components]] id = "amp"    kind = "winamp"  options = { players = ["firefox", "spotify"] }
[[components]] id = "temps"  kind = "sensors"

[[rules]] id = "gpu-hot"   when = "gpu.temp_c{0} > 85"   for = "10s"   clear = "gpu.temp_c{0} < 80"   severity = "crit"   title = "GPU hot"
```

```toml
# layout.toml — pages and placements only; `kind = "clock"` places an anonymous default-options instance
schema = 1
[grid] columns = 12   rows = 6   gap = 1   borders = "each"   cell_aspect = 0.5   min_unit_inner = { cols = 8, rows = 3 }

[[pages]] name = "Overview"   hotkey = "1"
place = [
  { id = "cpu",   at = [0, 0], size = [6, 3], priority = 100 },
  { id = "gpu",   at = [6, 0], size = [6, 3], priority = 100 },
  { id = "pins",  at = [0, 3], size = [4, 2], priority = 90 },
  { id = "lan",   at = [4, 3], size = [4, 2] },
  { id = "viz",   at = [8, 3], size = [4, 2] },
  { id = "amp",   at = [0, 5], size = [4, 1] },
  { id = "temps", at = [4, 5], size = [6, 1] },
  { kind = "clock", at = [10, 5], size = [2, 1] },
]
[[pages]] name = "Audio"   hotkey = "2"
place = [
  { id = "amp", at = [0, 0], size = [6, 3] }, { id = "viz", at = [6, 0], size = [6, 3] },
  { id = "cpu",   at = [0, 3], size = [12, 3], view = "meters" },   # preferred tier: keep the CPU strip light under the audio page even though `table` would fit
]
```

The shipped default grows per arc: arc 1 ships cpu + clock and swaps the `amp` slot for a `sources` tile until arc 6 (D37); placements of kinds that do not exist yet render placeholder chips. Theme file (retrowave excerpt; `modern.toml` uses the same schema with Catppuccin values, `borders.set = "rounded"`, `title.style = "plain"` and every flourish/effect false):

```toml
[meta]     name = "retrowave"   schema = 1   variant = "dark"   inherits = "base-dark"
[palette]  indigo = "#0b0324"   violet = "#7a3fb5"   pink = "#ff2975"   cyan = "#00f0ff"   purple = "#b967ff"
           orange = "#ff8b39"   mint = "#05ffa1"   sun = "#fede5d"   red = "#fe4450"   snow = "#efe9ff"   dusk = "#8a7fb0"
[colors]   bg = "$indigo"   surface = "#1a0b3d"   panel = "#241b2f"   border = "$violet"   border_focused = "$pink"
           title = "$snow"   text = "$snow"   text_muted = "$dusk"   text_ghost = "#3d2a63"   cursor = "$pink"
[colors.accent]    primary = "$pink"   secondary = "$cyan"   tertiary = "$purple"
[colors.severity]  ok = "$mint"   warn = "$sun"   crit = "$red"   info = "$cyan"
[colors.selection] fg = "#ffffff"   bg = "#3d1a63"
[gradients] load = ["$cyan", "$purple", "$pink", "$orange"]   temp = ["$cyan", "$purple", "$pink", "$red"]
            audio = ["$cyan", "$pink", "$sun"]   title = ["#f6f0ff", "#c8b8ff", "$pink", "$violet"]
[glyphs]   set = "unicode"   nerd = false   bar = "nine_levels"   chart_marker = "octant_if_vte"
[borders]  set = "double"   focused_set = "thick"   merge = "exact"
[title]    style = "gradient"   case = "upper"   bold = true
[widgets]  gauge = "line"   bars = "eighths"   sparkline = "eighths"   table_header = "reverse"   big_number = "quadrant"
[flourish] grid_floor = true   sun = true   big_clock = { pixel = "quadrant" }   marquee = true
[effects]  startup = { kind = "sweep_in", motion = "left_to_right", duration_ms = 600 }
           alert = { kind = "hsl_pulse", lightness = 25, period_ms = 900, target = "crit_fg" }
[components.audio] gradients.audio = ["$cyan", "$purple", "$pink"]
```

The showcase-class theme (`matrix.toml`, excerpt — arc 4):

```toml
[meta]     name = "matrix"   schema = 1   variant = "dark"   inherits = "base-dark"   class = "showcase"
[palette]  void = "#000000"   deep = "#020a04"   moss = "#0b3d16"   neon = "#00ff41"   mint = "#7dffa6"   leaf = "#009a2e"
           ghost = "#0a3312"   dim = "#1f9a4a"   paper = "#b6ffc9"   amber = "#e0c341"   pill = "#ff2a2a"
[colors]   bg = "$void"   surface = "$deep"   panel = "$deep"   border = "$moss"   border_focused = "$neon"
           title = "$mint"   text = "$paper"   text_muted = "$dim"   text_ghost = "$ghost"   cursor = "$neon"
[colors.accent]    primary = "$neon"   secondary = "$mint"   tertiary = "$leaf"
[colors.severity]  ok = "$neon"   warn = "$amber"   crit = "$pill"          # crit pierces the veil, BOLD|REVERSED
[gradients] load = ["$ghost", "$leaf", "$neon", "$mint"]   rain = ["#ffffff", "#d8ffe0", "$neon", "#00b32c", "#008f11", "#004d0a", "#002a05"]
[glyphs]   set = "unicode"   rain = "katakana"   bar = "nine_levels"   chart_marker = "octant_if_vte"
[borders]  set = "plain"   focused_set = "thick"   merge = "exact"
[title]    style = "plain"   case = "upper"
[flourish] decode = true   big_clock = { pixel = "quadrant" }
[ambient]  kind = "matrix_rain"   fps = 24   density = 0.20   speed = 1.0
           reveal = ["focus", "alert", "hover", "key"]   reveal_ms = 2500   governor = true
[ambient.light]    fade_s = 12   trail_ms = 900   sweep_s = 20   head = "#ffffff"   floor = "#000000"   relight_on_update = true
                   # printed content fades to `floor` (black) over fade_s; empty-space trails fade in trail_ms; a dense sweep re-prints everything every sweep_s
[effects]  startup = { kind = "rain_fill", duration_ms = 1200 }   focus = { kind = "decode", duration_ms = 400 }
           theme_swap = { kind = "dissolve", duration_ms = 600 }   alert = { kind = "hsl_pulse", period_ms = 900, target = "crit_fg" }
```

Hot reload (D53): the `gw-watch` thread `stat()`s `config.toml`, `layout.toml` and the active theme file (a `.toml` theme and the sibling it inherits; built-ins are embedded) once per second — mtime + size, no `notify`, immune to editor renames — and sends `ControlMsg::Reload { kind }`; the **shell** re-reads and parses on the render thread (the files are small), keeps instances whose `(kind, options)` are unchanged (their state survives), rebuilds the rest, drops removed ones, follows the pages/grid/fps, swaps the theme when the config's theme *name* changed (never when `--theme`/`NO_COLOR` locked it), and on a parse or validation error keeps the old state and toasts `file:line:col`. `T` reloads the theme on demand. Edit-mode saves (arc 4a, D54 seam 5) write **`layout.toml` only**, through `toml_edit::DocumentMut`: each `[[pages]]` table's `place` array is replaced by inline tables (`{ id = "cpu", at = [0, 0], size = [6, 3], view = "…", priority = N }`); every other key, comment and blank line survives — comments *inside* a `place` array and a comment on the array's closing `]` line do not, and a changed page count rebuilds the `pages` array. The write is atomic (temp + rename in the same dir), re-parsed beside the current `config.toml` and compared to memory before it counts, and its content hash reaches the watcher first (`WatchHandle::ignore_sender`), which skips that one change. `config.toml` is never written.

## 10. UX and keys

Global: `Ctrl-q` quit, `?` help, `F12` stats HUD (frame time p50/p95, changed cells, bytes written, mode). Grid mode: `1–9` pages, `[`/`]` prev/next, `Tab`/`Shift-Tab` reading-order focus, `h j k l`/arrows spatial focus, `Enter` capture keys into the focused component (its `keys` replace the status bar), `Esc`/`Outcome::Release` returns, `z` zoom, `d` dense (overrides the size-derived mode for the session), `t` cycle theme (every built-in), `T` reload theme (the file, or the built-in), `V` re-light the page and `L` lock everything lit (showcase themes; a toast explains them elsewhere), `V` re-light the page (showcase themes), `L` lock everything lit / unlock, `space` pause (Level::Paused; pins keeps sampling), `r` record toggle, `a` ack banner, `A` alerts tile, `S` screenshot to `~/.local/state/gridwatch/shot-<ts>.txt`, `e` edit mode, `q` quit when not captured. Mouse (SGR, opt-out; Shift-drag keeps native VTE selection): click focus, double-click zoom, wheel forwarded with local coordinates. Edit mode (`e`, arc 4a — D54 seam 1, amended by its review): `H J K L` move one unit; `Ctrl-l` widen, `Ctrl-h` narrow, `Ctrl-j` grow down, `Ctrl-k` shrink up (the legacy encoding cannot tell Shift-Ctrl from Ctrl, so the direction carries the sign; crossterm maps the bytes `0x08`/`0x0a` to `Ctrl-h`/`Ctrl-j` itself — the plain Backspace and Return keys do nothing in edit mode); `s` cycle `manifest.footprints`; `S` then `h/j/k/l` swap with the neighbour (`Esc` cancels the pending swap); `a` picker (unplaced instance ids, then every kind as `kind:<name>` → `insert_first_fit` at the default footprint, a 2x1 slot for a kind this build lacks; every plain letter filters, `↑/↓`/`Tab`/`Ctrl-n/p` move, the list scrolls with the cursor); `x` or `Delete` remove; `u`/`Ctrl-r` undo/redo (64 page snapshots, cleared on a page change); `w` save (`layout.toml unchanged — nothing to save` when nothing changed); `Esc` leave — `unsaved changes — w save · y discard · Esc stay` when dirty, and a page change asked for while dirty is taken after the answer, still in edit mode; a refused op leaves the page unchanged, draws the **attempted** rect as a red double-bordered ghost (Crit's BOLD|REVERSED, so mono can tell it from the green fit ghost) until the next key and says why in the key bar — the key bar is terse enough for 120 columns and a note replaces the key list, `?` has the long form; the gutters (and only the gutters: never a cell inside a tile, nothing in dense mode's shared borders or at `gap = 0`) show a dotted unit grid, and stack mode says on the bar that edits apply but are not drawn; a layout hot-reload under edit mode re-baselines the session and toasts; the alerts overlay closes on entry. Keys that keep working in edit mode: pages (`1-9`, `[`, `]`), `q`, `Ctrl-q`, `t`, `T`, `space`, `d`, `A`, `Tab`/`Shift-Tab`, `?`, `F12`; component capture does not; `S` is swap and `a` is add here, never screenshot/ack. Mouse: press focuses and starts a drag, the ghost previews (green fits, red not), release applies one undo step (a drag back to the start is a no-op, not an undo step); a press on the bottom-right corner cell resizes. Only the legacy keyboard encoding is assumed (VTE has no kitty protocol).

## 11. Error handling and degraded modes

- **Capability probe** (`CapSet`, ≤200 ms, per-check timeout, cheap checks only) + `Manifest.requires/optional`: a missing required capability skips `build` and installs a placeholder chip whose lines are the reason **and the fix** (`usermod -aG i2c`, udev rule, `apt install pipewire-bin`) from one table (`probe::explain`). `gridwatch doctor [--offline]` prints every capability with `✓/✗`, the reason and the fix, and runs the sources' live probes (`gridwatch_sources::doctor`: the exporter GET, `detect_bus`) unless `--offline` (D53).
- **Runtime loss** → `SourceStatus { state: Starting|Ok|Degraded|Unavailable|Stopped, reason, hint, since, last_sample, dropped, restarts }`; a tile whose *required* source's `last_sample` is older than 3 × the cadence that source runs at (configured `refresh_ms`/`interval_ms`, the pins source's live interval, else the registry default) is dimmed to `TextMuted`+`DIM` with a `STALE 12s` badge on its top border, in the shell's post-render pass (not while paused, not before the first sample, not for a source that is not `Ok`, not until a parked source's first sample after a resume; a finished replay counts real time — D53); the `sources` tile shows everything.
- **Terminal lifecycle.** The app never calls `ratatui::init`, `try_init`, `run` or `restore` (they install a hook that restores the terminal before any chained hook runs; astral-watch's `tui.rs` uses `ratatui::init()` and is therefore *not* the reference). The sequence is `enable_raw_mode()`, `execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, EnableFocusChange)`, `Terminal::new(CrosstermBackend::new(stdout()))`, and the mirror image on exit; `clippy.toml` lists those four functions under `disallowed-methods` so `-D warnings` catches a regression. The shell is generic: `run<B: Backend>(terminal: &mut Terminal<B>, …)`, so the determinism test drives the whole app through `TestBackend`.
- **Panic policy.** One hook consults a thread-local `PanicPolicy`: source threads are `Contained` (the supervisor's `catch_unwind` marks the source `Unavailable("panicked: …")` and restarts with backoff and a restart counter); the render thread is `Contained` only inside `catch_unwind` around `tick`/`render`/`on_key` of a single component (the instance becomes a placeholder chip and stops being called); everywhere else the hook restores the terminal and defers to color-eyre. Profile keeps `panic = "unwind"`.
- **No locks on the render thread**: single-writer store, atomics for demand, channels for everything else.
- **Focus.** The app starts *focused*; if the terminal never reports focus (DECSET 1004 unsupported — some tmux/ssh paths) it stays focused and `space` is the manual throttle. `FocusLost` handling only ever *reduces* work (D39).
- **Error visibility (D46).** Any failure *before* the alternate screen is entered is printed to the inherited stderr — the log does not exist from the user's point of view yet; any failure *after* is logged **and** surfaced in the UI where a surface exists (a source failure is a status on the `sources` tile and a toast on the transition to `Unavailable`). Pinned by `docs/TESTING.md` layers C and D.
- **stderr** is `dup2`'d to `$XDG_STATE_HOME/gridwatch/gridwatch.log` once the terminal is up (after the tty check and `enter`) (astral-watch's library `eprintln!`s would otherwise scribble on the UI); `tracing` goes to the same file.
- **Channels**: data bounded 4096 with `try_send` and drop counters (a 60 Hz audio publisher plus all pollers cannot fill it in under ~40 s of a stalled render thread; pause stops emission at the source); control unbounded and drained first, so an alarm raised during a stall is applied on the next frame and always reaches the journal; input unbounded and drained before both.

## 12. Testing, replay, demo

1. **Unit** (store, headless): `Ring`, `resample` (bucket alignment, gaps, aggs), `Store::apply` (generation, retention), `RuleEngine` (hold/clear, absent/NaN), `AlertLog`, journal round trip over every catalogued Record type, "alert emitted while the data channel is full is still applied and journaled", `tracks()` (exact sum, ≤1 variance) cross-checked against `Layout::horizontal(vec![Fill(1); n]).spacing(Spacing::Overlap(1))`, `thresholds()` for the shipped grids, htop/nvtop formulas against recorded `/proc` text, `nearest_256` on known hexes, DSP (full-scale sine → 0 dBFS; a 50 Hz sine → exactly one dominant low band), MPRIS metadata decoding from recorded `a{sv}` maps.
2. **Snapshots** (`insta 1.48`, `gridwatch-ui` testkit feature): `snapshot_matrix!(component, sizes)` snapshots the **view tree** (YAML) from `demo::Synth` at a fixed `Ts` for each tier — the semantic contract — and then renders it through the default renderer into `Buffer` at the real inner sizes the 12×6 grid produces on 250×70 (configured) and 120×40 (dense) plus canonical rects, with the `modern` theme only, and snapshots `dump::cells(&buffer)` — a run-length-encoded styled dump (fg/bg/modifiers per run). Themes are covered by one compact **role swatch** snapshot per theme (one line per Role, eight stops per gradient) plus targeted `cell().fg`/modifier assertions (e.g. Crit is REVERSED in `mono`). `assert_never_panics(component)` sweeps every inner size from 0×0 up to the richest tier's `min` plus the zoomed body (248×66 at 250×70) with `zoomed: true`; `assert_min_tier_fits(component, min_unit_inner)`; `assert_tiers_well_formed(component)` (mins monotone non-decreasing, `zoom_only` tiers form a suffix, at least one non-zoom tier); a layout test asserts the expected **tier name** per placement of the shipped default layout at 250×70 and 120×40, not merely non-chip. Showcase themes: a determinism test renders two frames of `matrix_rain` from the same seed and frame index and asserts identical buffers; a readability test renders the Overview under `matrix` with a synthetic pins overload across a full sweep cycle and asserts that the alerting tile, the focused tile and the banner are fully lit at every frame with no rain glyphs over them; a fade test lights one cell and asserts its colour walks the `Rain` LUT from head to `floor` within `fade_s` and then stops changing; a sweep test asserts every content cell of the page reaches `lit = 1` at least once per `sweep_s`; a re-light test applies a one-cell data change and asserts exactly that cell returns to `lit = 1` on the next ambient frame; a composition test asserts the mold is never composited directly — with the droplet pool emptied, a `matrix` frame contains nothing but pinned tiles, fading prints and fading trails.
3. **Property** (proptest 1.11): edit ops never overlap or leave bounds; `Store::apply` of shuffled batches yields the same latest values; mode selection is monotonic in terminal size with hysteresis.
4. **Replay**: `fixtures/journals/torch-*.jsonl` (60 s, tables off) drive `apply_all` then derived-fact assertions; a determinism test replays the same journal twice through the full `run<TestBackend>` on a virtual clock and asserts identical frame hashes; recorded input fixtures assert "keys in → commands out" (e.g. `F9`+`Enter` yields `Kill{SIGTERM}` only after confirmation).
5. **Demo mode**: `gridwatch --demo [--seed N]` swaps every source for its seeded `Synth` (32 cores with a game-like load, a 5090 at 400 W, a 1.5× idle pin imbalance with a scripted overload at t=40 s, a synthetic stereo mix, a fake Firefox track with embedded art). `gridwatch shot --theme retrowave --size 250x70 --page 1 --format ansi|cells|svg` renders headless from the synth, and `shot --replay FILE --at SECS` from a journal on the virtual clock (both byte-deterministic, D41); `dump::svg` is a hand-written SVG (one `<rect>` per background run, one `<text>` per foreground run, 8×16 px cells) and CI's `docs` job runs `scripts/shots.sh --check`, which regenerates `docs/img/*.svg`, `docs/KEYS.md` (`gridwatch keys`) and `docs/COMPONENTS.md` (`gridwatch component list`) and fails on drift (arc 2a).
6. **Hardware-gated** `#[ignore]` tests run manually on torch (pw-record frame count and quantum check via `pw-top -b -n 1`, NVML static probe vs spec row, `detect_bus`).
7. **Parity**: `docs/PARITY.md` lists every htop, nvtop and astral-watch feature as in-scope (arc N) or out (reason); the parity arc's acceptance is a diff against it.

## 13. Performance budget (250×70 Ptyxis, release)

The binding requirements — process CPU, wake-ups, bytes written to the terminal, the load imposed on Ptyxis (itself a GPU client), NVML call time, i2c bus time, memory and startup — and the measurement protocol are in `docs/PERFORMANCE.md`; they are acceptance gates per arc, and this section is the per-frame breakdown that has to add up to them.

These are **p95** ceilings: layout solve <50 µs (cached per page/size/zoom/mode); all `tick`s ≤1 ms; `view` construction ≤0.3 ms per visible tile; all visible `render`s ≤3 ms; overlay + effects ≤1 ms; ratatui diff + write ≤8 ms. The CPU budgets imply a *mean* frame cost of ≤ 3 ms at the Overview's ≈ 2 frames/s and ≤ 1.3 ms at 30 fps — which is what the render cache in §5 exists for: at 30 fps only the animated tile re-renders, the diff of 17 500 cells costs ~0.3 ms, and the write is the tile's cells. Whole-process CPU per page, measured and recorded in `docs/PERFORMANCE.md`: Overview with silent audio <2 % of one core beside a running game; Overview with the visualizer live at its 30 fps default <6 %; the Audio page at 60 fps opt-in <10 %. Showcase-class themes (`matrix`) are measured against the S-ceilings while focused and against P4 when not. RSS <60 MB (NVML maps ~20 MB; store ≤32 MB; audio ring 8192 frames/channel; art cache 8 × ≤256 px). The `F12` HUD ships in arc 1 so VTE throughput on the real layout is measured before 60 fps becomes a default anywhere.

## 14. Packaging and CI

`ci.yml` mirrors astral-watch (`dtolnay/rust-toolchain`, `Swatinem/rust-cache`): fmt, `clippy --workspace --all-targets --all-features -D warnings` (with the `clippy.toml` bans), `cargo test --workspace`, `cargo doc -D warnings`, release build `--locked`, MSRV 1.88 `cargo check --workspace --locked --all-features`, per-crate `cargo check -p gridwatch-store -p gridwatch-ui` (no feature unification), a feature matrix (`--no-default-features`, each source/component feature alone, all), `cargo deny check` (licences MIT/Apache-2.0/BSD/ISC/CC0/Zlib/Unicode/CDLA-Permissive-2.0 — ISC and CDLA are unavoidable via ring and webpki-roots under ureq/rustls; bans `cpal`, `mpris`, `pipewire`, `libpulse*`, `ansi_colours`, `enum-map`, `sysinfo`; a separate job asserts `tokio` is absent unless any of `mpris`, `net-probe`, `net-rdns`, `net-dns` is enabled), `cargo tree -d` duplicate guard for ratatui-core/crossterm, `cargo audit`, and (from arc 2) the headless demo screenshot job. `release.yml` on tags builds gnu + musl tarballs, nfpm deb/rpm with `Recommends: pipewire-bin, libnvidia-compute`, an AUR PKGBUILD and a Nix flake. Not published to crates.io until astral-watch 0.8.0 is; `cargo install --git` is documented. `CHANGELOG.md` gets one minor version per arc.

## 15. Adding a component

1. New data? Add typed keys (+ any Record type with `Serialize + Deserialize` and a `decode` entry) to `gridwatch-store/src/keys/<domain>.rs` with unit, doc and `SOURCE` constant; extend `gridwatch-store/src/demo/synth.rs` so demo and snapshots have the keys; add `gridwatch-sources/src/<domain>/` implementing `Source` (or `Sampler`, or `AsyncSource`) with a `SourceDef { start, demo }`; gate with a cargo feature.
2. Copy `components/src/clock.rs`; write `static MANIFEST` (sources and optional sources), `Options` (`#[serde(default, deny_unknown_fields)]`, view-only), `tiers()` poorest-first with `tiers[0].min ≤ 8×3` and `zoom_only` tiers last, `demand(tier)` if any tier needs a process scan or gated columns, `tick` (derive state when `store.generation(src)` changed), `view` per tier returning a `View` tree from `cx.store`/`cx.now` (reach for `Custom` only when no node fits, and then paint through theme roles), optional keys returning `Command`s.
3. Register `DEF` in `components/src/registry.rs` and the feature in `gridwatch/Cargo.toml`; one `#[cfg(feature)]` line in `builtin_registry()`; add the feature to the CI matrix.
4. Tests: `snapshot_matrix!`, `assert_renders_everywhere` (with a `signature(tier)` on the component, D46), `assert_min_tier_fits`, a replay assertion if a fixture exists; `cargo insta review`.
5. Add to `fixtures/layouts/showcase.toml` and `docs/PARITY.md` if it emulates a tool; `docs/COMPONENTS.md` is regenerated from `gridwatch component info <kind>`; CHANGELOG entry.

## 16. Resolved review concerns (round 2)

- **Journal cannot serialise records** (major): `Datum::Record(Arc<dyn RecordValue>)` with a blanket impl over `Serialize` types, `KeyMeta::decode` per Record type, `lookup()` interning of metric and source names, and a round-trip test over the whole catalogue (§4.1, §4.5).
- **Alerts on a lossy channel** (major): three channels; control (status/alert/done/reload) is unbounded, never dropped, drained before data; unit test for an alert raised while data is full (§4.2, §11).
- **min_terminal contradicts tiers** (major): thresholds derived from `GridSpec` + `min_unit_inner` (131×37 configured, 109×27 dense, stack below), dense triggered by terminal size with a 2-cell hysteresis and never by a starved cell, `tiers[0].min ≤ 8×3` enforced by test, 120×40 no-chip assertion in arc 1. `rows = "auto"` as default is **rejected**: placements are in fixed row units, so a changing row count would push placements out of bounds; the auto-rows formula is removed instead (§6).
- **Tier composition / view / SizeClass** (major): tiers are cumulative supersets with `adds`; `view` is a preferred tier with fallback + `view↓` chip; `footprint`, `SizeClass` and `Shape` are removed from the contract (`title` takes `max_width`) (§4.6).
- **Cross-arc data dependencies** (major): `Manifest.optional_sources` contributes to Demand; the cpu source reads Tccd by label in arc 1 and the sensors source takes the unchanged key over later; Winamp declares `audio` optional and draws a static vis when absent (§5, §8).
- **nvtop parity backwards, no definition** (major): gpu `charts` ships with the gpu component (arc 2); `docs/PARITY.md` per tool with in/out per arc; GPU keys labelled by device index from day one (§4.1, §8, §12).
- **Config sprawl** (major): singleton sources under `[sources.<id>]` only; instance options view-only with a disjointness test; `[[components]]` and `[[rules]]` live in `config.toml`; `layout.toml` holds pages/placements only; `kind:` shorthand placements; the net source collects everything and instances filter; `hide_kernel_threads` is a view option; sink changes are global by design (§9).
- **Testkit/demo direction** (major): `Synth` moved to `gridwatch-store::demo`; `SourceDef.demo` wraps it; no dev-dependency exception needed (§3, §15).
- **Arc 1 overscope** (blocker): arc 1 drops the GPU source/component, the journal, the process table, `inherits`/overrides/WCAG/flourishes/effects, and most CLI subcommands; those move to the arcs whose tiers first render them (roadmap). The reviewer's "modern/mono only" is **partially rejected**: `retrowave` stays in arc 1 because the meta-requirement is a visually impressive first arc and the trimmed loader (roles, `$palette`, gradients, glyph tiers, borders, titles) is what the bars need anyway; the htop `cores` tier (32 gradient-coloured bars in CCD blocks) stays for the same reason, the `table` tier moves out.
- **crossterm in store/ui** (two minors): `gridwatch_store::InputEvent` mirror converted once in the input thread; crossterm only in app and bin; input has its own channel drained first.
- **ratatui-core default = []** (minor): explicit feature pins and a per-crate `cargo check` job (workspace document, §14).
- **deny.toml licences and tokio assertion** (minor): ISC and CDLA-Permissive-2.0 allowed; tokio assertion rephrased over four features (§14).
- **Audio latency changes the quantum** (minor): default 1024, `low_latency` opt-in, acceptance rewritten with a `pw-top` check (§8, roadmap).
- **Bass resolution** (minor): cava-style dual FFT (8192 below 250 Hz, 2048 above), `audio.bands` documented as display resolution, 50 Hz sine test (§8, §12).
- **Wrong terminal-init reference** (minor): explicit sequence, no `ratatui::init/try_init/run/restore`, clippy `disallowed-methods` (§11).
- **Snapshot explosion** (minor): styled RLE dumps at `modern` only, per-theme role swatches, targeted cell assertions (§12).
- **Performance headline** (minor): `[sources.audio] fps = 30` default with 60 opt-in, DSP idles at 2 Hz on silence, per-page budgets stated (§5, §13).
- **Arc-internal inconsistencies** (minor): Chrome honoured in arc 1; README image via an ANSI dump plus a manually captured PNG until the SVG dumper lands in arc 2; `[flourish]`/`[effects]` ignored with a warning until the effects arc; `run<B: Backend>` is part of the arc-1 app contract.
- Kept unchanged from revision 1 (not challenged): PanicPolicy hook, 12-column grid, theme roles/gradients, tachyonfx bounding, demand levels, astral-watch pinning and upstream PRs, NVML tiers, procfs-only CPU data, zbus/halfblock MPRIS, 1 Hz mtime watcher, toml_edit save, MSRV 1.88 and the dependency bans.
