<!-- Architecture proposal (superseded by docs/ARCHITECTURE.md). Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Proposal: opstui — "one store, many views" (crate `opstui`, repo github.com/mbeaman/opstui)

_Angle: data-first_

**Philosophy.** Every pixel on screen is a pure function of a single, typed, timestamped telemetry store; every byte that enters that store arrives as a message on one channel, from a source that runs on its own thread and knows nothing about rendering. That inversion buys the three things a multi-session showcase project needs most: components become cheap (a new view over existing data is a `view(&Store, area, class, theme)` function and a snapshot test), consistency becomes structural (the same `Key<f64>` feeds a 1x1 big number, a 6x3 chart and an alert rule, with identical units and history), and testability becomes total (record the message stream on torch once, replay it in CI forever, render any component at any footprint and theme at a fixed virtual time, and diff the bytes). Sources are the only place I/O happens and the only place that can fail; they fail into a status the store carries and components render, so a missing GPU, i2c group, or PipeWire binary is a labelled tile, not a crash. Alerts are messages like any other, evaluated on ingest and routed to an overlay that is drawn last, on every page, driven by conditions rather than samples so it cannot flicker.

# opstui — architecture (data / reactive-first)

Target: Rust 1.88 (ratatui 0.30.2's MSRV), edition 2024, MIT, `github.com/mbeaman/opstui`. Everything below is grounded in the 2026-08-30 research digest for host "torch" (Ptyxis 50.1 / VTE 0.84, truecolor, no Nerd Fonts, no sixel/kitty graphics, no audio/dbus/alsa dev headers, user in `i2c`, RTX 5090 on driver 610, PipeWire 1.6.2).

## 1. Workspace layout

```
opstui/
├── Cargo.toml                  # workspace; [workspace.dependencies]; [patch] astral-watch → ../astral-watch for local dev
├── deny.toml                   # cargo-deny: licences (no LGPL/NC), duplicate-crate advisories
├── README.md  CHANGELOG.md  LICENSE  CONTRIBUTING.md
├── docs/                       # architecture.md, components.md, themes.md, journal.md (formats), keys.md (generated metric catalogue)
├── fixtures/
│   ├── journals/               # recorded on torch: idle.jsonl, game.jsonl, audio.jsonl, alert-overload.jsonl (synthetic)
│   ├── layouts/                # default.toml, showcase.toml, tiny-80x24.toml
│   └── themes/                 # third-party imports used by tests (base16 sample, alacritty sample)
├── crates/
│   ├── opstui-store/           # NO ratatui dependency. Ts/Clock, Key<T>, Store, ring buffers, Msg, Source trait, Demand, alerts, journal
│   │   └── src/{lib,ts,clock,key,ring,series,store,msg,source,demand,status}.rs
│   │       src/keys/{mod,cpu,gpu,pins,net,audio,media,sensor,sys}.rs        # the metric catalogue (typed constants)
│   │       src/alert/{mod,event,log,rule,engine}.rs
│   │       src/journal/{mod,format,record,replay}.rs
│   ├── opstui-ui/              # ratatui-core + ratatui-widgets. Theme, SizeClass, Component trait, layout engine, shared widgets, overlay, dumps
│   │   └── src/{lib,size,component,viewcx,uistate,overlay,dump}.rs
│   │       src/theme/{mod,file,color,gradient,glyphs,borders,effects,contrast}.rs  + builtin/{modern,retrowave,phosphor-green,mono}.toml
│   │       src/layout/{mod,grid,page,edit,focus}.rs
│   │       src/widgets/{stacked_bar,spectrum,scope,big_number,chip,toast,banner,grid_floor,sparkline_ext,kv_table}.rs
│   ├── opstui-sources/         # one module per source, each behind a cargo feature; depends only on opstui-store
│   │   └── src/{lib,registry,supervisor,backoff}.rs
│   │       src/demo/{mod,synth}.rs
│   │       src/cpu/{mod,stat,mem,psi,procs,topology,freq}.rs
│   │       src/gpu/{mod,nvml,specs,smi_fallback}.rs
│   │       src/pins/{mod,i2c,exporter,csv,lifecycle_bridge}.rs
│   │       src/net/{mod,dev,link,addrs,route,dns,conns,probe,wifi}.rs
│   │       src/audio/{mod,pwrecord,dsp,bands,scope,vu}.rs
│   │       src/mpris/{mod,proxy,discovery,art}.rs
│   │       src/sensors/{mod,hwmon,rapl}.rs
│   ├── opstui-components/      # one module per component; depends on opstui-store + opstui-ui
│   │   └── src/{lib,registry,clock,sources,alerts}.rs
│   │       src/{htop,gpu,pins,net,audio,winamp,sensors}/{mod,view_*,keys,state}.rs
│   └── opstui/                 # the binary: terminal, input thread, main loop, executor, config, CLI
│       └── src/{main,cli,app,run_loop,input,executor,paths}.rs
│           src/config/{mod,behaviour,layout,rules,watch}.rs
│           src/cmd/{run,record,replay,shot,check,theme,keys}.rs
├── packaging/{nfpm.yaml, aur/PKGBUILD, opstui.desktop}   # deb/rpm/AUR like astral-watch; Nix flake in a later arc
└── .github/workflows/{ci.yml, release.yml}
```

Dependency direction is strict and enforced by the crate graph: `store ← ui ← components ← bin`, `store ← sources ← bin`. `opstui-store` compiles in about a second with no TUI or system dependencies, so the entire data model, alert engine and journal are unit-testable headless. `opstui-ui` and `opstui-components` depend on `ratatui-core 0.1.2` + `ratatui-widgets 0.3.2` (the widget-library convention); the binary depends on `ratatui 0.30.2` (default crossterm 0.29 backend) — same types, so no duplication.

## 2. Core types (opstui-store)

### 2.1 Time

```rust
/// Nanoseconds since the run (or journal) epoch. Monotonic, serialisable, deterministic under replay.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ts(pub u64);

/// The only clock anyone may read. Real in normal runs, virtual under replay/tests.
#[derive(Clone)]
pub enum Clock { Real { start: Instant }, Virtual(Arc<AtomicU64>) }
impl Clock {
    pub fn now(&self) -> Ts;
    pub fn wall(&self, epoch: SystemTime, t: Ts) -> SystemTime;   // the clock tile shows *recorded* time under replay
}
```

### 2.2 Typed metric ids

```rust
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetricId { pub name: &'static str, pub label: Label }
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Label { None, Index(u16), Name(Arc<str>) }         // core=7, pin=3, iface="eno1", chip="k10temp:Tccd1"

/// Phantom-typed handle. `T` is the value type a reader gets back.
pub struct Key<T> { pub id: MetricId, _t: PhantomData<fn() -> T> }
impl<T> Key<T> {
    pub const fn new(name: &'static str) -> Self;
    pub fn idx(&self, i: u16) -> Self;           // gpu::FAN_PCT.idx(2)
    pub fn named(&self, s: &str) -> Self;        // net::RX_BPS.named("eno1")
}

pub trait Value: Send + Sync + 'static { const KIND: Kind; fn into_datum(self) -> Datum; fn from_datum(d: &Datum) -> Option<&Self>; }
#[derive(Clone, Copy)] pub enum Kind { Scalar, Vector, Record }
pub enum Datum { Scalar(f64), Vector(Arc<[f32]>), Record(Arc<dyn Any + Send + Sync>) }
```

`f64` is `Scalar` (keeps history), `Vec32` (= `Arc<[f32]>`) is `Vector` (latest + short history: audio bands, NVML 20 ms power trace), and any `'static` struct is `Record` (latest only: process table, link state, now-playing, GPU static info). The catalogue lives in `keys/*.rs` and is the shared vocabulary between sources, components and rules:

```rust
pub mod gpu {
    pub const UTIL_PCT: Key<f64>   = Key::new("gpu.util_pct");
    pub const MEMCTL_PCT: Key<f64> = Key::new("gpu.memctl_pct");     // NVML utilization.memory — NOT VRAM
    pub const VRAM_USED_B: Key<f64> = Key::new("gpu.vram_used_b");
    pub const POWER_W: Key<f64>    = Key::new("gpu.power_w");
    pub const POWER_TRACE: Key<Vec32> = Key::new("gpu.power_trace");  // samples(Power), 20 ms spacing
    pub const TEMP_C: Key<f64>     = Key::new("gpu.temp_c");
    pub const FAN_PCT: Key<f64>    = Key::new("gpu.fan_pct");         // .idx(fan)
    pub const THROTTLE: Key<f64>   = Key::new("gpu.throttle_bits");
    pub const INFO: Key<GpuInfo>   = Key::new("gpu.info");            // name, arch, cores, bus width, limits, thresholds, spec row
    pub const PROCS: Key<GpuProcTable> = Key::new("gpu.procs");
}
pub mod pins { pub const AMPS: Key<f64> = Key::new("pins.amps"); /* .idx(1..=6) */ pub const TOTAL_W: Key<f64> = ...; pub const BALANCE: Key<f64> = ...; }
pub mod audio { pub const BANDS: Key<Vec32> = Key::new("audio.bands"); /* .idx(0|1) L/R */ pub const SCOPE: Key<Vec32> = ...; pub const RMS_DB: Key<f64> = ...; pub const SINK: Key<SinkInfo> = ...; }
```

`opstui keys` prints the catalogue (name, kind, unit, producing source) — `docs/keys.md` is generated from it in CI so it cannot drift.

### 2.3 Series, ring buffers, resampling

```rust
pub struct Ring<T> { buf: Box<[MaybeUninit<T>]>, head: usize, len: usize }   // fixed capacity, O(1) push, no alloc after new
impl<T: Copy> Ring<T> { pub fn push(&mut self, v: T); pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T>; pub fn last(&self) -> Option<&T>; }

pub struct Retention { pub max_len: usize, pub max_age: Duration }          // default 2400 points / 10 min (nvtop parity)
pub struct ScalarSeries { pub unit: Unit, ring: Ring<(Ts, f64)>, pub min: f64, pub max: f64 }
pub struct VectorSeries { pub latest: (Ts, Arc<[f32]>), ring: Ring<(Ts, Arc<[f32]>)> }
pub struct RecordSeries { pub latest: (Ts, Arc<dyn Any + Send + Sync>) }

#[derive(Clone, Copy)] pub enum Agg { Last, Mean, Max, Min }
impl Store {
    pub fn last(&self, k: &Key<f64>) -> Option<(Ts, f64)>;
    pub fn window(&self, k: &Key<f64>, span: Duration) -> impl Iterator<Item = (Ts, f64)> + '_;
    /// Any scalar at any width: `buckets` slots ending at `now`, `None` where no sample landed.
    pub fn resample(&self, k: &Key<f64>, span: Duration, buckets: usize, agg: Agg) -> Vec<Option<f64>>;
    pub fn vector(&self, k: &Key<Vec32>) -> Option<(Ts, &Arc<[f32]>)>;
    pub fn record<T: 'static>(&self, k: &Key<T>) -> Option<(Ts, &T)>;
    pub fn labels(&self, name: &'static str) -> impl Iterator<Item = &Label>;   // enumerate ifaces / cores / pins present
    pub fn status(&self, s: SourceId) -> &SourceStatus;
    pub fn alerts(&self) -> &AlertLog;
    pub fn generation(&self, s: SourceId) -> u64;                                // dirty tracking for the render gate
}
```

`resample` is what makes "effortless new views" real: a sparkline of GPU power at whatever width the cell has is `store.resample(&gpu::POWER_W, 60s, area.width as usize, Agg::Max)`, and the same call at 20 buckets feeds a 1x1 tile.

### 2.4 Messages and the single-writer rule

```rust
pub struct Sample { pub id: MetricId, pub datum: Datum }
pub struct Batch { pub source: SourceId, pub at: Ts, pub samples: SmallVec<[Sample; 8]> }
pub enum Msg {
    Batch(Batch),
    Status(SourceId, SourceStatus),
    Alert(AlertEvent),                 // domain alerts (astral-watch Lifecycle) already debounced upstream
    Input(crossterm::event::Event),    // from the input thread
    CommandDone(CommandId, Result<String, String>),
    Reload(Reload),                    // theme / layout / rules from the file watcher
    Tick,
}
impl Store {
    /// The ONLY mutation. Applies a message, updates generations, runs alert rules on the touched metrics,
    /// returns the alert events it produced (already merged into `self.alerts`).
    pub fn apply(&mut self, msg: &Msg) -> SmallVec<[AlertEvent; 2]>;
}
```

The store is owned by the app on the render thread; the ingest step drains the channel and calls `apply` per message. There is no lock anywhere in the data path. Recording is therefore trivial (tee the channel to a file), replay is trivial (feed the same messages back), and `apply` is a pure state transition that property tests can hammer.

### 2.5 Sources and demand

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub struct SourceId(pub &'static str);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] #[repr(u8)] pub enum Level { Hidden = 0, Visible = 1, Focused = 2 }

pub struct SourceCtx {
    pub id: SourceId, pub tx: SyncSender<Msg>, pub clock: Clock, pub stop: Arc<AtomicBool>,
    level: Arc<AtomicU8>,                     // written by the app after every layout solve
    pub ctl: Receiver<Control>,               // options / restart / domain commands from the executor
    pub options: toml::Table,
}
impl SourceCtx {
    pub fn level(&self) -> Level;
    pub fn emit(&self, at: Ts, samples: impl IntoIterator<Item = Sample>);   // try_send; counts drops into status
    pub fn status(&self, s: SourceStatus);
    pub fn sleep(&self, d: Duration) -> bool;                                 // interruptible (200 ms steps), false when stopped
}
pub struct SourceInfo { pub id: SourceId, pub produces: &'static [&'static str], pub cadence: Cadence, pub needs: &'static [Requirement] }
pub struct Cadence { pub hidden: Option<Duration>, pub visible: Duration, pub focused: Duration }   // None = pause when hidden

/// Blocking sources: run on their own std thread.
pub trait Source: Send + 'static {
    fn info(&self) -> SourceInfo;
    fn run(self: Box<Self>, cx: SourceCtx);          // loop until cx.stop; never panics out (supervisor restarts with backoff)
}
/// Async-native sources (zbus, surge-ping, hickory): run on the single shared tokio runtime thread.
pub trait AsyncSource: Send + 'static {
    fn info(&self) -> SourceInfo;
    fn run(self: Box<Self>, cx: SourceCtx) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}
pub enum Control { SetOption(String, toml::Value), Restart, Domain(Box<dyn Any + Send>) }
```

**Threading model and why.** Blocking sources (procfs parsing, NVML which blocks 1–21 ms per call, astral-watch's i2c ioctls that busy-poll for 4–33 ms, the `pw-record` pipe reader, the FFT) run as std threads exactly like astral-watch does; tokio would only wrap them in `spawn_blocking` for no gain. zbus 5.19 (MPRIS), surge-ping (ICMP) and hickory-resolver (rDNS) are async-native, so they share one `tokio` runtime on one thread (`Builder::new_current_thread`), feature-gated with `mpris`/`net-probe`. Both kinds funnel into one `std::sync::mpsc::sync_channel::<Msg>(4096)`; sources `try_send` and count drops. The render thread owns the `Terminal`, the `Store`, UI state and tachyonfx effects (which are `!Send`), and drives the loop with `recv_timeout` — the single-threaded get-event→update→render shape the ratatui FAQ recommends. Demand is a per-source `AtomicU8` the app writes after each layout solve; sources read it once per tick to pick their cadence (GPU fast tier 250 ms when visible, 1 s hidden; pins never below 500 ms and never paused because alerts depend on them; audio kills `pw-record` after 5 s hidden and respawns on demand).

### 2.6 Alerts as first-class events

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub enum Severity { Info, Warn, Crit }
#[derive(Clone, PartialEq, Eq, Hash)] pub struct AlertId(pub Arc<str>);          // "pins/overload", "rule/gpu-hot"
pub enum Transition { Raised, Repeated, Resolved }
pub struct AlertEvent { pub id: AlertId, pub source: SourceId, pub severity: Severity, pub transition: Transition,
                        pub title: Arc<str>, pub detail: Arc<str>, pub at: Ts }
pub struct Active { pub event: AlertEvent, pub since: Ts, pub acked: bool }
pub struct AlertLog { active: BTreeMap<AlertId, Active>, ring: Ring<AlertEvent> /* 500 */ }
impl AlertLog { pub fn active(&self) -> impl Iterator<Item = &Active>; pub fn recent(&self, since: Ts) -> impl Iterator<Item = &AlertEvent>; pub fn ack(&mut self, id: &AlertId); pub fn worst(&self) -> Option<Severity>; }

/// rules.toml → typed rule; evaluated inside Store::apply for every scalar the batch touched.
pub struct Rule { pub id: AlertId, pub metric: MetricId /* label may be Any */, pub cond: Cond, pub hold: Duration, pub clear_hold: Duration,
                  pub severity: Severity, pub title: String, pub detail: String /* "{value:.0} °C on {label}" */ }
pub enum Cond { Gt(Rhs), Ge(Rhs), Lt(Rhs), Le(Rhs), Absent(Duration) }
pub enum Rhs { Lit(f64), Metric(MetricId, f64 /* offset */) }      // "gpu.temp_c >= gpu.slowdown_c - 5"
pub struct RuleEngine { rules: Vec<Rule>, pending: HashMap<(AlertId, Label), Ts> }
impl RuleEngine { pub fn observe(&mut self, store: &Store, touched: &[MetricId], now: Ts) -> SmallVec<[AlertEvent; 2]>; }
```

Two producers, one log: domain alerts arrive as `Msg::Alert` (the pins source runs astral-watch's `Lifecycle` — 3-of-5 confirm, telemetry-lost freeze — and maps `Overload|Disconnected|Imbalance → Crit`, `ImbalanceAdvisory → Warn`, `TelemetryLost → Info`); generic threshold rules (`gpu.temp_c`, `cpu.temp_c{Tctl}`, `net.link{eno1} absent`, `sensor.temp_c{nvme*}` vs `_crit`) are evaluated on ingest with `hold`/`clear_hold` hysteresis. The overlay keys on `AlertId` transitions, never on samples, so it cannot flicker; `hold` is the only debounce for rules and none is added for domain alerts.

### 2.7 Journal: record, replay, demo

```rust
pub struct Header { pub version: u16, pub wall_epoch: SystemTime, pub host: String, pub sources: Vec<SourceInfo>, pub terminal: (u16, u16) }
pub struct Recorder { w: BufWriter<File>, tables: bool }       // JSON Lines: header, then one line per Msg::{Batch,Status,Alert}
impl Recorder { pub fn open(path: &Path, header: &Header, tables: bool) -> io::Result<Self>; pub fn tee(&mut self, msg: &Msg) -> io::Result<()>; }
pub struct Replay { clock: Arc<AtomicU64>, speed: f64, msgs: Vec<(Ts, Msg)>, pub header: Header }
impl Replay {
    pub fn load(path: &Path) -> Result<Self>;
    pub fn into_source(self, speed: f64) -> ReplaySource;            // a Source that advances the virtual clock as it emits
    pub fn apply_all(self, store: &mut Store) -> Ts;                  // tests: no timing, returns the final Ts
    pub fn apply_until(self, store: &mut Store, t: Ts);
}
```

`opstui run --record ~/rec.jsonl` tees every data message (input events excluded by default; `--record-input` includes them for full UI replays). `opstui run --replay rec.jsonl --speed 4` runs the real UI on a virtual clock. Records (process tables) are the bulk of a file: a 10-minute torch recording is ~15 MB with tables, ~1 MB without; `--tables off` keeps CI fixtures small and a later arc adds zstd. `opstui run --demo` swaps every source for `demo::Synth` (seeded xorshift, no RNG dependency): CPU waves per CCD, a game-like GPU load ramp, six pins at 1.5 A ± jitter with a scripted overload at t=40 s, a stereo band mix with beats, bursty network, 400 fake processes, a fake Firefox MPRIS track with art from an embedded PNG. Demo is how README screenshots are made and how the UI is developed without touching hardware.

## 3. UI core (opstui-ui)

### 3.1 Size classes and footprints

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] pub struct Footprint { pub w: u8, pub h: u8 }   // grid units
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub enum SizeClass { Tiny, Small, Medium, Large, Huge }
impl SizeClass {
    /// From the INNER rect (after borders). width <12/<24/<48/<96, height <3/<6/<12/<24, class = min(w, h).
    pub fn of(inner: Size) -> Self;
}
#[derive(Clone, Copy)] pub enum Shape { Wide, Tall, Squarish }
```

Nominal footprints (1x1, 2x1, 4x2, 6x3) pick a curated look via the picker and `s` (cycle size); rendering always keys on the real inner `Rect` — a 6x3 on an 80-column terminal is a `Medium`, and the component must not care.

### 3.2 The component contract — pure functions of the store

```rust
pub struct ViewCx<'a> {
    pub store: &'a Store, pub area: Rect /* inner */, pub class: SizeClass, pub shape: Shape,
    pub theme: &'a Theme, pub focus: Focus /* None | Grid | Component */, pub zoomed: bool, pub dense: bool,
    pub now: Ts, pub wall: SystemTime, pub ui: &'a dyn Any /* this instance's UiState */, pub options: &'a toml::Table,
}
pub enum Outcome { Ignored, Consumed, Command(Command), ReleaseFocus }
pub trait Component: Send + Sync {
    fn kind(&self) -> &'static str;
    fn title(&self, class: SizeClass, cx: &ViewCx) -> Cow<'static, str>;
    fn needs(&self) -> &'static [SourceId];                        // drives demand; missing → placeholder from SourceStatus
    fn footprints(&self) -> &'static [Footprint];
    fn min_size(&self, class: SizeClass) -> Size;
    fn priority(&self) -> u8 { 50 }
    fn init_state(&self, options: &toml::Table) -> Box<dyn Any + Send>;   // scroll/sort/selection; NO data
    fn view(&self, cx: &ViewCx, buf: &mut Buffer);                        // pure: no I/O, no locks, no time reads
    fn on_key(&self, key: KeyEvent, ui: &mut dyn Any, store: &Store) -> Outcome { Outcome::Ignored }
    fn on_mouse(&self, ev: MouseEvent, local: Position, ui: &mut dyn Any, store: &Store) -> Outcome { Outcome::Ignored }
    fn keymap(&self) -> &'static [KeyHint] { &[] }
    fn fps_hint(&self, cx: &ViewCx) -> Option<u8> { None }        // audio asks for 60 when focused
}
/// Anything with side effects is data handed to the executor thread — never done inside a component.
pub enum Command { Kill { pid: i32, signal: i32 }, Renice { pid: i32, nice: i32 }, Affinity { pid: i32, cpus: Vec<u16> }, IoPrio { pid: i32, class: u8, level: u8 },
                   Media(PlayerCmd), AudioTarget(String), SetSourceOption(SourceId, String, toml::Value), SaveLayout, Record(bool), Ack(AlertId), Quit }
```

Components own no data: history is in the store, per-instance UI state (sort key, selected PID, scroll offset, DSP preset) is a `Box<dyn Any>` the app keeps per instance and hands back on every call. Because `view` reads `cx.now`, not `Instant::now()`, a component rendered under replay at a fixed `Ts` produces identical bytes every time.

### 3.3 Theme

```rust
#[derive(Clone, Copy, EnumCount, EnumIter)] pub enum Role { Bg, Surface, Panel, Border, BorderFocused, Title, Text, TextMuted, TextGhost,
    AccentPrimary, AccentSecondary, AccentTertiary, Ok, Warn, Crit, Info, SelectionFg, SelectionBg, Cursor }
#[derive(Clone, Copy)] pub enum GradientId { Load, Temp, Power, Mem, NetRx, NetTx, Audio, Title }
#[derive(Clone, Copy)] pub enum ColorMode { TrueColor, Ansi256, Ansi16, Mono }
pub struct Gradient { lut: [Color; 64], shade: [&'static str; 5] }        // Oklab-interpolated (palette 0.7.7), downsampled at load
impl Gradient { pub fn at(&self, t: f32) -> Color; pub fn shade_at(&self, t: f32) -> &'static str; }
pub struct Glyphs { pub set: GlyphSet /* Ascii | Unicode | Nerd(opt-in) */, pub bar: &'static [&'static str] /* NINE_LEVELS */, pub marker: Marker, pub ok: &'static str, pub warn: &'static str, pub crit: &'static str, pub arrows: [&'static str; 4] }
pub struct Theme { pub meta: Meta, colors: [Color; Role::COUNT], gradients: EnumMap<GradientId, Gradient>, pub glyphs: Glyphs,
                   pub borders: Borders, pub title: TitleSpec, pub flourish: Flourish, pub effects: Effects, pub mode: ColorMode }
impl Theme {
    pub fn load(file: &ThemeFile, parent: Option<&ThemeFile>, mode: ColorMode) -> Result<Self, ThemeError>;   // $palette, inherits, contrast gate (WCAG via palette::Wcag21RelativeContrast)
    pub fn color(&self, r: Role) -> Color; pub fn style(&self, r: Role) -> Style; pub fn gradient(&self, g: GradientId) -> &Gradient;
    pub fn severity(&self, s: Severity) -> (Style, &str);            // colour + glyph + BOLD|REVERSED for Crit; works in Mono
    pub fn block<'a>(&self, title: Line<'a>, focused: bool, dense: bool) -> Block<'a>;
    pub fn for_component(&self, kind: &str) -> Arc<Theme>;            // pre-merged [components.<kind>] override
}
pub fn detect_mode(cli: Option<ColorMode>) -> ColorMode;              // CLI > NO_COLOR (→ Mono theme) > COLORTERM > TERM
```

Themes are TOML files in `~/.config/opstui/themes/` plus four embedded built-ins (`modern` = Catppuccin Mocha, `retrowave` = Synthwave '84 + `#ff2975` with the digest's contrast fixes, `phosphor-green`, `mono`). Components never name a `Color`; they ask for a `Role`, a `GradientId` or a glyph. Effects are named hooks (`startup`, `theme_swap`, `focus_change`, `alert_enter`, `alert_active`, `ambient`) resolved to tachyonfx 0.25.1 effects at load; ambient CRT is off even in retrowave and gated by a `budget_ms` watchdog.

### 3.4 Layout engine

```rust
pub struct GridSpec { pub columns: u8 /* 24 */, pub rows: Rows /* Fixed(6) | Auto */, pub gap: u8, pub borders: BorderMode /* Each | Shared | None */, pub cell_aspect: f32, pub min_terminal: Size }
pub struct Placement { pub id: InstanceId, pub at: [u8; 2], pub size: [u8; 2], pub priority: u8 }
pub struct Page { pub name: String, pub hotkey: Option<char>, pub rows: Option<u8>, pub place: Vec<Placement> }
pub struct Cell { pub id: InstanceId, pub outer: Rect, pub inner: Rect, pub class: SizeClass, pub starved: bool }
pub enum SolveMode { Grid, Dense, Stack, TooSmall }
pub struct Solved { pub mode: SolveMode, pub cells: Vec<Cell>, col_starts: Vec<u16>, row_starts: Vec<u16> }
impl LayoutEngine {
    pub fn tracks(len: u16, n: u8, gap: u16) -> Vec<(u16, u16)>;     // pure integer; widths differ by ≤1; exact sum
    pub fn solve(&self, spec: &GridSpec, page: &Page, body: Rect, zoom: Option<&InstanceId>, min_of: &dyn Fn(&InstanceId, SizeClass) -> Size) -> Solved;
    pub fn hit(&self, s: &Solved, pos: Position) -> Option<&Cell>;
    pub fn cell_at(&self, s: &Solved, body: Rect, pos: Position) -> Option<(u8, u8)>;
}
// edit ops are pure and property-tested: never two overlapping placements after any op sequence
pub fn move_by(p: &Page, id: &InstanceId, dx: i8, dy: i8, spec: &GridSpec) -> Result<Page, EditError>;
pub fn resize_by(p: &Page, id: &InstanceId, dw: i8, dh: i8, spec: &GridSpec, min: Footprint) -> Result<Page, EditError>;
pub fn swap(p: &Page, a: &InstanceId, b: &InstanceId) -> Result<Page, EditError>;
pub fn insert_first_fit(p: &Page, id: InstanceId, fp: Footprint, spec: &GridSpec) -> Result<Page, EditError>;
pub fn focus_dir(s: &Solved, from: &InstanceId, dir: Dir) -> Option<InstanceId>;   // spatial: overlap projection, min edge distance
```

Fixed 24-column unit grid per page (rows default 6, `auto` from terminal aspect); the same integer function that lays out is inverted for mouse hit-testing; shared-border mode extends spans by one cell and uses `Block::merge_borders(MergeStrategy::Exact)`. Degradation ladder: configured → dense → starved cells become chips (`▪ gpu`) → below `min_terminal` a priority-ordered stack with a scroll offset.

### 3.5 Overlay

```rust
pub struct Overlay;   // drawn LAST every frame on every page: banner (top row, Crit, theme alert_active pulse), toasts (bottom-right, 8 s), status chips (Info), edit-mode ghost grid, help popup, frame-stats HUD
impl Overlay { pub fn view(&self, cx: &OverlayCx, buf: &mut Buffer); }
pub struct OverlayCx<'a> { pub store: &'a Store, pub theme: &'a Theme, pub area: Rect, pub now: Ts, pub mode: &'a Mode, pub stats: Option<&'a FrameStats> }
```

## 4. Runtime (the `opstui` binary)

Threads: **input** (sole caller of `crossterm::event::read()`, forwards `Msg::Input`), **one per blocking source** (named `src-cpu`, `src-gpu`, `src-pins`, `src-net`, `src-audio-io`, `src-audio-dsp`, `src-sensors`), **one tokio thread** for async sources (`src-async`: mpris, net-probe, rdns), **executor** (runs `Command`s: signals, renice, MPRIS calls via `Control::Domain`, layout saves through `toml_edit`), **watcher** (notify 8.2 on `~/.config/opstui/`, 250 ms debounce → `Msg::Reload`), and the **render thread** (main): owns `Terminal`, `Store`, `App`, effects. Supervisor: every source thread runs inside `catch_unwind`; a panic or `run` returning early sets `SourceStatus::Degraded` and restarts with backoff 250 ms → 5 s (pw-record exits on PipeWire restarts; NVML `LibRmVersionMismatch` is shown, not retried).

```rust
pub struct App { store: Store, clock: Clock, theme: Arc<Theme>, layout: LayoutConfig, page: usize, mode: Mode, zoom: Option<InstanceId>,
                 instances: HashMap<InstanceId, Instance /* kind, options, Box<dyn Any> ui state */>, sources: Vec<SourceHandle>,
                 effects: EffectManager<&'static str>, solved: Solved, last_gen: HashMap<SourceId, u64>, stats: FrameStats, recorder: Option<Recorder> }

fn run_loop(term: &mut DefaultTerminal, app: &mut App, rx: &Receiver<Msg>) -> Result<()> {
    let mut next_frame = app.clock.now();
    loop {
        let ingest_deadline = Instant::now() + Duration::from_millis(3);
        while let Ok(msg) = rx.try_recv() {                       // 1. ingest everything queued (bounded)
            if let Some(r) = &mut app.recorder { r.tee(&msg)?; }
            app.handle(msg);                                      //    Input → focus/edit/component on_key → Command → executor; else store.apply
            if Instant::now() > ingest_deadline { break; }
        }
        app.solve_layout(term.size()?);                           // 2. layout → demand levels → sources
        let period = Duration::from_millis(1000 / app.target_fps());   // 30 default; 60 when a component asks
        if app.clock.now() >= next_frame {                        // 3. render only when something changed
            if app.dirty() || app.effects.is_running() { term.draw(|f| app.view(f))?; app.stats.frame(); }
            next_frame = app.clock.now() + period;
        }
        match rx.recv_timeout(until(next_frame)) { Ok(m) => app.handle(m), Err(RecvTimeoutError::Timeout) => {}, Err(_) => return Ok(()) }
        if app.quit { return Ok(()); }
    }
}
```

Tick rates (all per-source, all demand-aware): cpu meters 500 ms focused / 1.5 s visible (htop default) / 3 s hidden, process table 1.5 s only while a footprint that shows it is visible, `smaps_rollup` only when PSS columns are on; gpu fast tier 250 ms (100 ms focused), slow tier 1 s, PCIe from byte-counter fields never `pcie_throughput`; pins 500 ms / 1 s hidden, never paused; net counters 1 s (250 ms + EWMA when focused), link/addrs every 5 s, conns 2 s only when visible, probes 1 Hz; audio: `pw-record` 512-frame chunks (~10.7 ms) → DSP at ≤60 Hz publishing only when a vis component is visible; mpris event-driven + 1 Hz position poll while Playing; sensors 1 Hz; demo 250 ms. `dirty()` is `store.generation(src) != last_gen[src]` for any source a visible component needs, or an active overlay/effect, or a 1 Hz heartbeat for the clock.

## 5. How the seven components plug in

| Component | Source(s) & thread | Metrics written (catalogue) | Footprints | Commands |
|---|---|---|---|---|
| **htop** | `cpu` (procfs 0.18 `default-features=false`, hand-rolled deltas keyed by `(pid, starttime)`) | `cpu.total_pct`, `cpu.core_pct{core}`, `cpu.breakdown{core}` (Record `CpuBreakdown` nice/user/kernel/irq/softirq/steal/guest/iowait), `cpu.freq_mhz{core}`, `cpu.topology` (die_id, SMT pairs), `mem.{used,shared,buffers,cache,available,total,swap_used,swap_cache}` (htop formulas), `psi.{cpu,mem,io}.some10`, `tasks.*`, `sys.load{1,5,15}`, `sys.uptime_s`, `proc.table` (Record `ProcTable`, `Arc<[ProcRow]>` sorted by PID) | 1x1 big-number + sparkline; 2x1 stacked bars; 4x2 per-CCD core blocks (SMT pairs side by side, Tccd temps from `sensor.temp_c{k10temp:Tccd1}`); 6x3 + top-N table; Full = htop parity (screens, tree, search/filter, F-bar) | `Kill/Renice/Affinity/IoPrio` via executor (nix 0.31 signal+sched, libc setpriority/ioprio_set), gated by `readonly` and a confirm line |
| **gpu** | `gpu` (nvml-wrapper 0.12.1 on `src-gpu`; static probe once; per-field `NotSupported` → stop polling that field) | `gpu.{util_pct,memctl_pct,vram_used_b,vram_total_b,temp_c,power_w,power_limit_w,gclk_mhz,mclk_mhz,pstate,throttle_bits,enc_pct,dec_pct,fan_pct{i},fan_rpm{i},pcie_rx_bps,pcie_tx_bps,energy_j}`, `gpu.power_trace` (Vector, 20 ms samples), `gpu.info` (name, arch, cores, bus width, thresholds via field IDs 193/194/196 with `temperature_threshold` fallback, hand-verified spec row keyed by PCI id 0x2B85), `gpu.procs` (v3 lists + `process_utilization_stats(now - tick)`; CPU%/RSS joined from `proc.table` in the component, not re-scanned) | 1x1 `GPU 19%` + temp badge; 2x1 gauges GPU/VRAM + clocks/power/temp line; 4x2 nvtop header parity + 20 ms power sparkline; 6x3 + rolling charts (`resample` over 10 min) + spec column; Full + process table with Power sub-panel showing pins beneath board power | `Kill` |
| **pins** | `pins` (astral-watch git rev `dce7eee`, `default-features=false`; `PinSource` auto: exporter 127.0.0.1:9942 → direct i2c → CSV tail; `Lifecycle` from `config::load(None)` runs in the source) | `pins.amps{1..6}`, `pins.volts{1..6}`, `pins.total_a`, `pins.total_w`, `pins.balance`, `pins.max_v`, `pins.info` (bus, PCI, model via `cards::gpu_at`, source kind); `Msg::Alert` for every `Event` | 1x1 big watts + balance badge + alert glyph; 2x1 six eighth-block bars with 9.2 A `┄`; 4x2 bars + numbers + balance gauge; 6x3 + watts sparkline + log; Full = tui.rs parity (Braille trend chart, 300-sample history from the store) | none (read-only by design); `Ack` |
| **net** | `net` (procfs `dev_status` + sysfs link + `/proc/net/route` + resolve1 over zbus + conns from `/proc/net/*` joined with own `/proc/*/fd`; `net-probe` async: surge-ping DGRAM ICMP → TCP-connect fallback) | `net.{rx_bps,tx_bps,rx_pps,tx_pps,rx_drop,tx_drop}{iface}`, `net.link{iface}` (Record: operstate, speed, duplex, carrier flaps, wifi dBm/bitrate via neli-wifi feature), `net.addrs`, `net.route` (gateway/dev), `net.dns`, `net.conns` (Record `ConnTable`), `net.{rtt_ms,loss_pct,jitter_ms}{target}` | 1x1 `↓ ↑` rates + link dot; 2x1 + rx/tx sparklines + SSID/speed; 4x2 iface table + mirrored Braille chart + probe strip; 6x3 + top connections; Full + sortable conn table, rDNS toggle | none in Tier 0; per-process bandwidth deferred to a capability-gated helper arc |
| **audio** | `audio` = supervisor spawning `pw-record --format f32 --rate 48000 --channels 2 --raw --latency 512 --target auto -P '{ stream.capture.sink = true, node.passive = true, node.name = "opsTui audio" }' -` (io thread → rtrb 0.4 SPSC → dsp thread: realfft 3.5 N=2048 Hann, log bars 30 Hz–16 kHz, dBFS floor −65, tilt +4 dB/oct, attack/release EMA, selectable winamp/cava gravity + peaks) | `audio.bands{0,1}` (Vector 64 in 0..1), `audio.peaks{0,1}`, `audio.scope` (Vector 512 f32), `audio.rms_db{0,1}`, `audio.peak_db{0,1}`, `audio.sink` (Record: node.name, description, serial, rate, channels, state via `pw-dump` every 2 s while visible) | 1x1 VU pair or 8 thin bars; 2x1 scope (Canvas `Marker::Octant` on VTE ≥0.78 else Braille) or mono spectrum; 4x2+ mirrored stereo spectrum (`⌊(w+1)/3⌋` thick bars, NINE_LEVELS rows, gradient `Audio`, `▔` peaks) + VU strip + sink name; Full + DSP knobs | `AudioTarget(sink)`, `SetSourceOption("dsp.*")`; asks `fps_hint = 60` when focused |
| **winamp** | `mpris` (async: hand-rolled zbus 5.19 Player/root proxies, `Position` uncached, discovery by `arg0ns` match rule, per-player owner-changed; art decoded on `spawn_blocking`, image 0.25, pre-encoded per size class with ratatui-image 11 `default-features=false` → halfblocks) | `media.players` (Record list), `media.now` (Record `NowPlaying`: title/artist/album/status/`pos_us`+`read_at`/`len_us: Option`/rate/can_* /trackid), `media.art` (Record `ArtSet` {class → Protocol}), `media.history` (local playlist from metadata transitions) | 1x1 status glyph + marquee row; 2x1 shade mode (marquee + time + 8-bar mini spectrum from `audio.bands`); 4x2 main window (tui-big-text Quadrant digits, marquee 220 ms steps from `cx.now`, kbps/kHz from `audio.sink`, 19-band spectrum, posbar, volume, transport row); 6x3 main + art or EQ (EQ weights the visualizer bands, persisted per theme); Full + playlist pane | `Media(PlayPause|Next|Prev|SeekRel|SeekAbs|Volume|Raise|Select)` |
| **sensors** | `sensors` (hwmon walker keyed by chip name + device path, labels from `*_label`, drop `_max > 1e6`; PSI; RAPL only if `energy_uj` readable → else status hint) | `sensor.temp_c{chip:label}` for k10temp Tctl/Tccd1/Tccd2, 3× nvme, 2× spd5118, 2× r8169, mt7925; `sensor.meta{..}` (max/crit); `rapl.pkg_w`; `psi.*` | 1x1 hottest sensor + trend arrow; 2x1 top-4 chips; 4x2 table with mini sparklines grouped by chip; 6x3 + Braille chart of selected sensors; Full | none |

Plus three tiny components that fall out of the store for free: `clock` (big-text wall time from `cx.wall`), `sources` (a table of `SourceStatus`: state, cadence, last-sample age, dropped — the debugging view), and `alerts` (the log as a scrollable list with ack).

## 6. Config files

`~/.config/opstui/config.toml` (behaviour), `layout.toml` (grid + instances + pages), `rules.toml` (alerts), `themes/*.toml`. All TOML via `toml 1.1.4` + serde with `deny_unknown_fields` and `Error::span()` for `file:line:col`; layered defaults ← file ← `OPSTUI_*` env ← CLI by hand (no figment: it pins toml 0.8).

```toml
# config.toml
theme = "retrowave"           # or "modern", "phosphor-green", "mono", or a file in themes/
fps = 30                      # 60 opt-in; components may request 60 while focused
color = "auto"                # auto | always | never | 16 | 256 | truecolor
mouse = true
readonly = false              # blanks kill/renice like `htop --readonly`
confirm_kill = true
[record] tables = false        # keep --record files small
[sources.gpu] fast_ms = 250
[sources.pins] source = "auto"  exporter = "127.0.0.1:9942"  csv = "/var/log/astral-watch/gpu-pins.csv"  interval_ms = 500
[sources.audio] latency = 512  fft = 2048  bars = 64  lo_hz = 30  hi_hz = 16000  floor_db = -65  tilt_db_oct = 4  gravity = "winamp"
[sources.net] show = ["en*", "wl*", "wg*", "tun*"]  hide = ["veth*", "br-*", "docker*", "virbr*"]  probes = ["gateway", "1.1.1.1", "8.8.8.8"]  rdns = false  public_ip = false
```

```toml
# layout.toml
schema = 1
[grid] columns = 24  rows = 6  gap = 1  borders = "each"  min_terminal = { cols = 80, rows = 24 }
[[components]] id = "cpu"   kind = "htop"
[[components]] id = "gpu"   kind = "gpu"
[[components]] id = "pins"  kind = "pins"
[[components]] id = "lan"   kind = "net"     options = { interface = "eno1" }
[[components]] id = "vis"   kind = "audio"
[[components]] id = "amp"   kind = "winamp"
[[components]] id = "temps" kind = "sensors"
[[pages]] name = "Overview"  hotkey = "1"
[[pages.place]] id = "cpu"   at = [0, 0]   size = [12, 3]  priority = 100
[[pages.place]] id = "gpu"   at = [12, 0]  size = [12, 3]  priority = 100
[[pages.place]] id = "pins"  at = [0, 3]   size = [6, 3]   priority = 90
[[pages.place]] id = "lan"   at = [6, 3]   size = [6, 3]
[[pages.place]] id = "vis"   at = [12, 3]  size = [8, 3]
[[pages.place]] id = "amp"   at = [20, 3]  size = [4, 3]
[[pages]] name = "Media"  hotkey = "2"
[[pages.place]] id = "amp"   at = [0, 0]   size = [16, 6]
[[pages.place]] id = "vis"   at = [16, 0]  size = [8, 6]
```

```toml
# rules.toml — generic threshold alerts; astral-watch alerts arrive already debounced from the pins source
[[rule]] id = "gpu-hot"     metric = "gpu.temp_c"        when = ">= gpu.slowdown_c - 5"  for = "5s"  clear = "10s"  severity = "warn"  title = "GPU near slowdown"  detail = "{value:.0} °C"
[[rule]] id = "cpu-hot"     metric = "cpu.temp_c{Tctl}"  when = ">= 90"                  for = "5s"  severity = "warn"  title = "CPU Tctl high"
[[rule]] id = "nvme-crit"   metric = "sensor.temp_c{nvme*}"  when = ">= sensor.crit_c - 5"  for = "10s"  severity = "crit"  title = "NVMe near critical"
[[rule]] id = "link-down"   metric = "net.rx_bps{eno1}"  when = "absent 5s"              severity = "info"  title = "eno1 stopped reporting"
```

The theme file schema (roles, `$palette`, `inherits`, gradients, glyph tiers, borders, title style, flourish, effects hooks, `[components.<kind>]` overrides) is the one in the digest; `opstui theme import` maps alacritty/wezterm/base16 files onto roles.

## 7. Keyboard, mouse, edit mode

Global (grid focus): `q`/`Ctrl-C` quit · `1–9` page · `[` `]` prev/next page · `Tab`/`Shift-Tab` reading-order focus · `h j k l`/arrows spatial focus · `Enter` give focus to the component (it then owns keys; `Esc` returns) · `z` zoom toggle · `d` dense toggle · `t` cycle theme · `T` reload theme · `space` pause ingest (store freezes; sources keep running and queue) · `R` toggle recording · `a` ack banner · `A` alert log · `S` sources panel · `F` frame-stats HUD · `?` help · `e` edit mode. Component focus uses each tool's native bindings (htop's `</> I P M T N t / \ F K H u a i F7 F8 F9 k`; nvtop's `F6 +/- F9`; winamp `x c v b z` transport, `←/→` seek 5 s, `+/-` volume, `p` player cycle; audio `m` mode, `g` gravity, `[ ]` range; net `d` rDNS, `a` all ifaces). Edit mode: `HJKL` move, `Ctrl-hjkl` resize, `s`+dir swap, `a` picker (kinds + footprints, first-fit insert), `x` remove, `u`/`Ctrl-r` undo/redo (page snapshots), `w` save via `toml_edit` (comments preserved, atomic rename, self-write hash ignored by the watcher), `Esc` leave. Mouse (crossterm SGR, opt-in `mouse = true`, undone in a chained panic hook): click focus, double-click zoom, wheel forwarded with local coordinates, drag move / corner-drag resize in edit mode. Keys are the legacy encoding only (VTE has no kitty keyboard protocol); `supports_keyboard_enhancement()` gates nothing essential.

## 8. Error handling and degraded modes

```rust
pub enum State { Starting, Ok, Degraded, Unavailable, Stopped }
pub struct SourceStatus { pub state: State, pub reason: Option<Arc<str>>, pub hint: Option<Arc<str>>, pub since: Ts, pub last_sample: Option<Ts>, pub dropped: u64, pub restarts: u32 }
```

The app never refuses to start because a source is missing; `opstui` on a laptop with no GPU, no i2c, no PipeWire simply shows labelled tiles. Concrete cases, all verified against torch's digest: `Nvml::init` → `LibloadingError` ⇒ `Unavailable("libnvidia-ml.so.1 not found", hint "install the NVIDIA driver")` and the `nvidia-smi` CSV fallback is tried at 1–2 s only for this case; `LibRmVersionMismatch` ⇒ `Degraded("driver/library mismatch — reboot")`, no retry; per-field `NotSupported` ⇒ that key is never written (components render `n/a`); pins `detect_bus` → `PermissionDenied` ⇒ hint "add yourself to the i2c group"; `NoTelemetry` (idle GPU answers zeros) ⇒ `Degraded("waiting for telemetry (GPU idle?)")` with `TelemetryLost` fed to the lifecycle so no false all-clear; `pw-record` missing ⇒ `Unavailable(hint "apt install pipewire-bin")`; child exits ⇒ restart with backoff 250 ms→5 s, >250 ms without data under `node.passive` ⇒ decay bars as silence; no MPRIS names ⇒ `Ok` with `media.now = None` and the winamp tile shows an idle skin; ICMP `EPERM` ⇒ TCP-connect probes with a "tcp" chip; RAPL `energy_uj` 0400 ⇒ `rapl.pkg_w` absent with hint (udev rule documented); `/proc/pressure/irq` absent ⇒ key absent. Stderr is `dup2`'d to `$XDG_STATE_HOME/opstui/opstui.log` at startup because astral-watch's library `eprintln!`s would scribble on the alternate screen. Sources run under `catch_unwind`; the render thread installs color-eyre's hook chained after ratatui's restore and `DisableMouseCapture`.

## 9. Testing strategy

1. **Unit** (`opstui-store`, headless, fast): `Ring` (push/wrap/iter), `resample` (bucket alignment, `None` gaps, all `Agg`s), `Key`/`Label` hashing, `Store::apply` (generation bumps, retention), `RuleEngine` (hold/clear hysteresis, metric-vs-metric RHS, label wildcards, `absent`), `AlertLog` (raise/repeat/resolve/ack), journal round-trip (`Msg` → line → `Msg`), `LayoutEngine::tracks` (exact sum, ≤1 difference) cross-checked against `Layout::horizontal(vec![Fill(1); 24]).spacing(Spacing::Overlap(1))`.
2. **Snapshot** (insta 1.48): for every component × every footprint bucket (`Tiny`…`Huge` at canonical rects 10x3, 20x5, 40x10, 80x20, 160x40) × every built-in theme, render into `Buffer::empty(rect)` with a fixed `Ts` from a store seeded by a fixture journal, and snapshot `dump::ansi(&buffer)` — the ANSI dump carries colours, so theme regressions are caught, unlike `TestBackend`'s text-only `Display`. `cargo insta review` is the workflow.
3. **Replay** (`fixtures/journals/*.jsonl` recorded on torch with `--tables off`): `Replay::apply_all` then assert derived facts (CPU% within 0–3200, six pins present, GPU info name, alert `pins/overload` raised at the scripted timestamp in the synthetic fixture); a **determinism test** replays the same journal twice through the full `App` on a virtual clock with a `TestBackend` and asserts identical frame hashes per tick; **UI replays** with `--record-input` fixtures drive `on_key` sequences and assert the emitted `Command`s (e.g. `F9`+`Enter` in htop yields `Kill{SIGTERM}` only after the confirm line).
4. **Property** (proptest): edit ops never produce overlapping or out-of-bounds placements; every component renders every `Rect` from 0x0 to 12x6 without panicking; `Store::apply` of shuffled batches yields the same latest values; `SizeClass::of` is monotonic.
5. **Demo mode as a test**: `opstui shot --demo --theme retrowave --size 200x60 --page 1 --at 45s --out docs/img/overview.svg` renders headless through the in-tree SVG/ANSI dumpers (`dump.rs`, ~150 lines, no deps); CI regenerates the README images and fails if `git diff` is non-empty, so screenshots never rot.
6. **Bench** (criterion, dev-only): `Store::apply` for a 636-row `ProcTable`, `resample` at 240 buckets, full-frame render at 250x70 per theme, `Overlay::view` with a pulse effect.
7. **Hardware-gated integration** (`#[ignore]`, run manually on torch): 0.5 s `pw-record` capture has ≥ 20 000 frames; NVML static probe matches the spec row; pins `detect_bus` finds the IT8915FN.

## 10. Performance budget (250x70, release, Ptyxis)

Render: layout solve < 50 µs (integer, no solver); component `view`s ≤ 3 ms total; overlay + effects ≤ 1 ms (effects area-scoped, `budget_ms = 4` watchdog auto-disables ambient ones); ratatui diff + write ≤ 8 ms at 30 fps for the dashboard, with the audio spectrum region (~120x30 changed cells/frame) the only thing allowed to run at 60. Ingest: ≤ 3 ms per frame (bounded loop), `ProcTable` apply is an `Arc` swap. Sources: whole-process CPU < 2 % of one core at defaults while a game runs (procfs full scan ~30–60 ms every 1.5 s, NVML fast tier < 20 µs, i2c block read ~4 ms every 500 ms, FFT N=2048 stereo ~40 µs at 60 Hz). Memory: ~200 scalar series × 2400 points × 16 B ≈ 8 MB plus records; hard cap 32 MB via retention. The `F` HUD shows frame time, changed cells and bytes written so the VTE throughput question is answered in arc 1 rather than guessed.

## 11. Packaging and CI

`ci.yml` mirrors astral-watch: fmt → clippy `--all-targets --all-features -D warnings` → `cargo test --workspace` (insta in CI mode) → `cargo doc -D warnings` → release build `--locked`; an **MSRV 1.88** job (`cargo check --locked --workspace --all-features`); a **feature matrix** job (`--no-default-features` with only `demo`, then each source feature alone: `cpu`, `gpu`, `pins`, `net`, `net-probe`, `audio`, `mpris`, `sensors`) proving every combination builds with only build-essential + pkg-config; `cargo deny check` (licences: MIT/Apache/BSD/CC0/Zlib only, so `ansi_colours` LGPL and NC code can never enter); `cargo tree -d` guard for duplicate ratatui-core/crossterm; a screenshot-freshness job. `release.yml` on tags: gnu + musl tarballs (nvml-wrapper dlopens at runtime, zbus is pure Rust, so musl works), nfpm deb/rpm, AUR PKGBUILD update, GitHub release with `CHANGELOG.md` section; Nix flake in the packaging arc. Arcs map to minor versions with a `CHANGELOG.md` entry each; the `[patch]` to `../astral-watch` is in a git-ignored `.cargo/config.toml` so committed builds resolve the pinned git rev.

## 12. Adding a new component (or a new view over existing data)

1. If new data is needed: add typed keys to `opstui-store/src/keys/<domain>.rs` with unit and doc comment; add or extend a source under `opstui-sources/src/<domain>/` implementing `Source` (or `AsyncSource`), declaring `SourceInfo { produces, cadence, needs }`, emitting `Batch`es via `cx.emit`, reporting `SourceStatus` on every failure path, and honouring `cx.level()`; register it in `sources/registry.rs` behind a cargo feature; extend `demo::Synth` so the keys exist in demo mode.
2. Create `opstui-components/src/<kind>/mod.rs` with a unit struct implementing `Component`: declare `needs`, `footprints`, `min_size`; write `view` as a match on `cx.class` calling small `view_tiny/small/medium/large/huge` functions that only read `cx.store` (`last`, `resample`, `record`) and `cx.theme`; keep UI state in a struct returned by `init_state`; return `Command`s from `on_key`, never act.
3. Register the kind in `components/registry.rs` (`kind → &'static dyn Component`).
4. Add a snapshot test module: render at the five canonical rects × built-in themes from `fixtures/journals/idle.jsonl` and the demo store; run `cargo insta review`.
5. Add the kind to `fixtures/layouts/showcase.toml` and, if it should alert, a rule in `rules.toml`.
6. Document it in `docs/components.md`; `cargo run -- keys` shows the new metrics; CI regenerates `docs/keys.md` and the README screenshot.

A brand-new view over existing data (say a "power" tile combining `gpu.power_w`, `pins.total_w` and `rapl.pkg_w`) is steps 2–4 only: no thread, no I/O, no lifecycle — about a hundred lines and a snapshot.

## Key decisions

- **Store ownership and concurrency**: The Store is owned by the render thread and mutated only by `Store::apply(&Msg)` in the ingest step; sources write through a single bounded `std::sync::mpsc::sync_channel<Msg>(4096)` and never touch the store. — Single-writer removes every lock from the data path, makes recording a channel tee and replay a channel feed, and turns the whole data model into a pure state transition that headless tests and proptests can drive. Latency cost is one frame (≤33 ms), which is below every source cadence except audio, and audio's 60 Hz bands still arrive fresher than the terminal can paint.
- **Threads vs tokio**: Blocking sources (procfs, NVML, astral-watch i2c, pw-record pipe + FFT, hwmon) run on dedicated std threads; async-native sources (zbus MPRIS, surge-ping, hickory rDNS) share one current_thread tokio runtime behind cargo features; the render loop is std `recv_timeout`. — The digest verifies every collector but D-Bus/ICMP/DNS is a blocking syscall (NVML 1–21 ms, i2c 4–33 ms), tachyonfx effects are !Send, and the ratatui FAQ recommends the single-threaded loop; tokio therefore buys nothing for the core and is confined to where crates require it. This also matches astral-watch's proven std-thread style.
- **Typed metric catalogue**: Metrics are `Key<T>` constants (`Scalar` f64 with ring history, `Vector` Arc<[f32]> latest+short history, `Record` Arc<dyn Any> latest) with `Label::{None, Index, Name}`, defined once in `opstui-store/src/keys/` and printed by `opstui keys`. — Sources, components and alert rules share one vocabulary with units, so a 1x1 tile, a 6x3 chart and a rule cannot disagree; `Store::resample(key, span, buckets, agg)` makes any scalar drawable at any width, which is the mechanism behind 'effortless new views'. Records avoid serialising process tables into scalars while staying type-safe via downcast.
- **Components as pure functions**: `Component::view(&self, cx: &ViewCx, buf)` reads only `cx.store`, `cx.theme`, `cx.now`; per-instance UI state is an app-owned `Box<dyn Any>`; side effects are returned as `Command` values executed by an executor thread. — Rendering at a fixed virtual `Ts` from a replayed store is byte-for-byte deterministic, so insta snapshots per footprint × theme and full-app determinism tests are cheap; input handling becomes testable as 'keys in, commands out'; and no component can block the frame or hold a handle to hardware.
- **Record / replay / demo share one path**: A JSON Lines journal of `Msg`s with a wall-clock epoch header; `Replay` drives a virtual `Clock` shared by sources, the loop and `ViewCx`; `demo::Synth` is just another `Source` with a seeded xorshift. — One mechanism yields CI fixtures recorded on torch, deterministic UI tests, a headless `shot` command that regenerates README images in CI, and hardware-free development of every component; JSONL keeps fixtures inspectable and needs no new dependency (serde_json is already in the tree via astral-watch).
- **Alerts**: Alerts are `Msg::Alert` events keyed by `AlertId` with Raised/Repeated/Resolved transitions; domain alerts come pre-debounced from astral-watch's `Lifecycle` inside the pins source, generic ones from a `rules.toml` engine run inside `Store::apply` with hold/clear hysteresis; an `Overlay` drawn last on every page renders active Crit as a banner and events as toasts. — Keying the overlay on condition transitions rather than samples prevents flicker; reusing the Lifecycle keeps on-screen alarms identical to the astral-watch service's; evaluating rules on ingest means any catalogue metric is alertable with no component involvement, and the pins source never pauses so the banner appears on any page.
- **Demand-driven cadence**: The app writes a per-source `Level::{Hidden, Visible, Focused}` atomic after each layout solve; each source declares `Cadence { hidden: Option<Duration>, visible, focused }` and reads the level once per tick. — Keeps whole-process CPU under 2 % beside a running game: process tables and connection scans only run while a footprint that shows them is visible, `pw-record` is killed when no visualizer is on screen, GPU polls at 250 ms only when visible; the lock-free atomic costs nothing and pins stay at ≥500 ms even when hidden because alerts depend on them.
- **Layout model**: Fixed 24-column unit grid per page with integer track mapping (no constraint solver), size classes computed from the real inner Rect, degradation ladder configured → dense/shared borders → placeholder chips → priority stack; ratatui `Layout` is used only inside components and for chrome. — Every dashboard product converged on unit grids; integer tracks are deterministic, invertible for mouse editing and trivially property-tested; classes from the real Rect keep a 6x3 on an 80-column terminal readable. Edit-mode saves go through toml_edit so hand-written comments survive.
- **Theme system**: Semantic roles + Oklab gradient LUTs + glyph tiers (ascii/unicode/nerd opt-in) + border/title/flourish specs + named tachyonfx effect hooks, in TOML with `$palette`, `inherits` and per-component overrides; colour capability resolved once (NO_COLOR → a real Mono theme); WCAG contrast gate at load; hot reload via notify 8.2 watching the directory. — Components never name colours, so one render path yields both 'modern' (Catppuccin Mocha) and 'retrowave'; crossterm silently drops all colour under NO_COLOR so a Mono theme with glyphs/REVERSED is required; VTE draws box/block/sextant/octant glyphs itself but no Nerd Font exists, so unicode tier is restricted to U+2500–259F/25xx and braille.
- **Crate boundaries and dependency policy**: Five workspace crates (`store` with zero TUI/system deps, `ui` on ratatui-core/widgets, `sources` behind per-source features, `components`, `bin` on ratatui 0.30.2); MSRV 1.88; no sysinfo (MSRV 1.95), no cpal/pipewire/libpulse/mpris crates (headers absent), no figment (toml 0.8 dup), no ansi_colours (LGPL); astral-watch pinned to git rev dce7eee with default-features=false. — The store crate compiles and tests in seconds and is where most logic lives; feature-gated sources guarantee `cargo build` works with only build-essential + pkg-config and the CI feature matrix proves it; the exclusions are each verified failures or licence risks in the digest; the astral-watch pin is unavoidable because the crate is not on crates.io and HEAD's API differs from v0.7.0.
- **Audio capture**: Default backend is a supervised `pw-record … --raw -` subprocess (f32/48k/stereo, `stream.capture.sink`, `node.passive`, latency 512) feeding an rtrb SPSC ring and an in-process realfft DSP thread; the pure-Rust `pulseaudio` 0.3 crate is the first in-process upgrade behind `audio-native`. — Verified end-to-end on torch today with no headers or packages, follows default-sink changes via WirePlumber, and never wakes idle DACs; cpal cannot build without libasound2-dev and pipewire-rs needs libpipewire headers plus clang.
- **GPU data**: nvml-wrapper 0.12.1 on a dedicated thread with three polling tiers, PCIe throughput from byte-counter field IDs 197/198, VRAM and MEMCTL shown as distinct keys, a hand-verified const spec table keyed by PCI id 0x2B85, nvidia-smi only as a LibloadingError fallback. — Measured costs show `pcie_throughput` blocks 21 ms per direction while counters return in 0.3 ms; nvtop's MEM bar (43 %) and NVML memory utilisation (5 %) are different quantities; gpuwatch's SQLite DB mislabels the 5090's PCI id so it must not be embedded.

## Proposed first arc

Arc 1 = opstui 0.1.0 "the loop is real": prove the reactive core end-to-end on torch with two real sources, one synthetic source, two components, two themes, and the record/replay/shot tooling that every later arc will build on. Concretely, in one session: (1) workspace skeleton with the five crates, `Cargo.toml` pins (ratatui 0.30.2, edition 2024, rust-version 1.88), `deny.toml`, and `ci.yml` (fmt, clippy -D warnings, test, doc -D warnings, MSRV 1.88, feature matrix with `demo` only / `cpu` / `gpu`); (2) `opstui-store` complete: `Ts`/`Clock`, `Key<T>`/`Label`/`Datum`, `Ring`, `ScalarSeries`/`VectorSeries`/`RecordSeries` with retention, `Store::{apply,last,window,resample,record,status,generation}`, `Msg`/`Batch`, `Source`/`SourceCtx`/`Cadence`/`Level`, `SourceStatus`, `AlertEvent`/`AlertLog`/`RuleEngine` with hold/clear, JSONL `Recorder`/`Replay`, and the `cpu`/`gpu`/`sys`/`pins` key catalogue with `opstui keys`; (3) `opstui-ui`: `SizeClass`, `Footprint`, `Component`/`ViewCx`/`Outcome`/`Command`, `Theme` loader with `modern` and `retrowave` built-ins, `ColorMode` detection and Mono fallback, gradients, glyph tiers, the integer grid engine with pages/focus/zoom/dense (edit mode deferred to arc 2), `Overlay` with banner/toasts/help/frame-stats HUD, `StackedBar`/`BigNumber`/`Chip` widgets, and the ANSI/SVG dumpers; (4) sources `demo::Synth` (all catalogue keys, scripted GPU-temp spike), `cpu` (procfs: /proc/stat per-class breakdown with htop's guest subtraction, meminfo formulas, loadavg, PSI, topology from sysfs, process table at 1.5 s gated by demand) and `gpu` (NVML static probe, fast/slow tiers, power trace, process list with utilisation, NotSupported pruning, LibRmVersionMismatch/Libloading degraded states, nvidia-smi fallback); (5) components `htop` (1x1, 2x1, 4x2 per-CCD blocks, 6x3 with read-only top-N table) and `gpu` (1x1, 2x1, 4x2 nvtop-header parity with 20 ms power sparkline), plus `clock` and `sources`; (6) the binary: input thread, supervisor with backoff and catch_unwind, run loop with dirty gating at 30 fps, config/layout/rules loading with spans, stderr redirect, `opstui run [--demo|--replay F --speed N|--record F]`, `opstui shot`, `opstui config check`; (7) tests: store unit tests, layout tracks vs ratatui oracle, rule hysteresis, insta snapshots for both components × five rects × two themes from the demo store, a replay determinism test on a torch-recorded `fixtures/journals/idle.jsonl` (tables off), proptest that no component panics on any Rect ≤ 12x6; (8) a measured Ptyxis throughput number from the HUD at 250x70 written into docs/performance.md. Adversarial review targets for this arc: channel backpressure under a 636-row ProcTable at 500 ms focused cadence, generation/dirty correctness (no stale frames, no busy redraw), and NotSupported handling on the 5090. Deferred to arcs 2–6: edit mode + toml_edit save, pins (astral-watch git dep + Lifecycle bridge + alert banner on every page), audio (pw-record + realfft spectrum), net (Tier 0 + probes), mpris/winamp (zbus + halfblock art), sensors, tachyonfx hooks, theme import, packaging/release workflow and Nix flake.
