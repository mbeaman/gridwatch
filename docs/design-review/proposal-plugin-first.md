<!-- Architecture proposal (superseded by docs/ARCHITECTURE.md). Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Proposal: patchbay (working title: opsTui) — repo github.com/mbeaman/patchbay, binary `patchbay`

_Angle: plugin-first_

**Philosophy.** A patch bay routes many signals into one panel of modules; patchbay does the same for a workstation: every data source is a *service* that publishes serde-able snapshots, every tile is a *component* that only renders into an inner Rect it is handed, and the app shell owns everything in between (terminal, grid, focus, theme, effects, config). The contract between those three layers is the product. It is small enough to hold in your head (one trait, one manifest, one feed type), strict enough that a component cannot reach a device, a thread, or a border glyph directly, and data-shaped enough that recording, replay, demo mode, snapshot tests and a future sandboxed/out-of-process component all fall out of the same seam. The app is itself a library, so "adding a third-party component" means writing a 30-line `main.rs` that registers one more `ComponentDef` — no dynamic loading, no ABI, no WASM until an ecosystem actually exists. Static Cargo features select what ships; capability probing at startup decides what runs; nothing that is missing on a machine (no GPU, no i2c, no PipeWire, no D-Bus) is ever an error, only a labelled degraded tile.

# patchbay — architecture

## 1. Workspace layout

```
patchbay/
├── Cargo.toml                 # [workspace], [workspace.dependencies] (single pin per crate),
│                              # [patch."https://github.com/mbeaman/astral-watch"] for local dev
├── rust-toolchain.toml        # channel = "stable"; workspace rust-version = "1.88" (ratatui 0.30.2)
├── deny.toml                  # cargo-deny: MIT/Apache/BSD/CC0/Zlib only; ban duplicate ratatui-core
├── Makefile · README.md · CHANGELOG.md · CONTRIBUTING.md · LICENSE (MIT)
├── crates/
│   ├── core/          patchbay-core        THE CONTRACT: Component, Manifest, Registry, Service/Feed,
│   │                                       TelemetryStore, Capability, Health, Layout engine, Theme
│   ├── app/           patchbay-app         shell: terminal, frame loop, pages/focus/zoom, edit mode,
│   │                                       config load/watch/save, effects (tachyonfx), alert overlay,
│   │                                       recorder/replayer, `pub fn run(Registry, Cli) -> Result<()>`
│   ├── cli/           patchbay             the binary: feature-gated registry assembly + clap subcommands
│   ├── testkit/       patchbay-testkit     TestBackend harness, snapshot matrix, proptest strategies,
│   │                                       synthetic + replay helpers (dev-dependency everywhere)
│   ├── services/                           data layer — one crate per source family
│   │   ├── procs/     patchbay-svc-procs   procfs 0.18: /proc/stat, meminfo, loadavg, PSI, process table
│   │   ├── nvml/      patchbay-svc-nvml    nvml-wrapper 0.12.1 worker, const spec table, nvidia-smi tier
│   │   ├── pins/      patchbay-svc-pins    astral-watch (git rev) i2c | exporter | CSV sources + Lifecycle
│   │   ├── net/       patchbay-svc-net     /proc/net/dev, sysfs link, routes, resolved, conn table, probes
│   │   ├── audio/     patchbay-svc-audio   pw-record supervisor, pw-dump enumeration, realfft DSP
│   │   ├── mpris/     patchbay-svc-mpris   zbus 5 blocking proxies, player supervisor, art fetch/decode
│   │   └── hwmon/     patchbay-svc-hwmon   hwmon walker, cpufreq, RAPL (privilege-gated)
│   └── components/                         view layer — one crate per component family
│       ├── clock/     patchbay-comp-clock  template component (tui-big-text clock; needs no service)
│       ├── cpu/       patchbay-comp-cpu    htop family
│       ├── gpu/       patchbay-comp-gpu    nvtop family + GPU-Z specs + Power sub-panel
│       ├── pins/      patchbay-comp-pins   12V-2x6 pin monitor (astral-watch parity)
│       ├── net/       patchbay-comp-net
│       ├── audio/     patchbay-comp-audio  spectrum / oscilloscope / VU
│       ├── media/     patchbay-comp-media  Winamp-style MPRIS now-playing
│       └── sensors/   patchbay-comp-sensors
├── themes/            retrowave.toml modern.toml phosphor-green.toml phosphor-amber.toml mono.toml
├── layouts/           default.toml showcase.toml            (embedded with include_str!, also copied
│                                                            to ~/.config/patchbay/ on first run)
├── recordings/        torch-2026-08-30/{procs,nvml,pins,net,hwmon,audio,mpris}.jsonl  (60 s, replay tests)
├── docs/              ARCHITECTURE.md COMPONENTS.md THEMES.md LAYOUT.md ADDING-A-COMPONENT.md
├── packaging/         nfpm/nfpm.yaml, AUR PKGBUILD, notes on udev (RAPL) and setcap (net helper, later)
└── .github/workflows/ ci.yml release.yml
```

**Crate boundaries.** `patchbay-core` depends on `ratatui-core 0.1.2` + `ratatui-widgets 0.3.2`, `serde`, `toml 1.1`, `palette 0.7.7`, `arc-swap`; it has no threads of its own beyond what `Feed` needs and never touches the terminal. Service crates depend on core plus their system crates (procfs, nvml-wrapper, zbus, astral-watch…), never on ratatui. Component crates depend on core, ratatui-core/-widgets and the *service crates they consume*, never on the app. `patchbay-app` depends on `ratatui 0.30.2` (facade, crossterm 0.29 backend), `tachyonfx 0.25.1`, `notify 8.2` + `notify-debouncer-full 0.7`, `toml_edit 0.25`. Only `patchbay` (cli) knows the full list of components — via Cargo features:

```toml
[features]
default = ["cpu", "gpu", "pins", "net", "audio", "media", "sensors", "clock"]
cpu = ["dep:patchbay-comp-cpu"]     # each feature = one component crate (+ its services transitively)
pins = ["dep:patchbay-comp-pins"]   # pulls astral-watch (git); `--no-default-features --features cpu,gpu`
# ...                               # builds a binary without it
```

`cargo tree -d` runs in CI so a second `ratatui-core`/`crossterm` can never sneak in (tui-big-text, tachyonfx and tui-bar-graph all pin ratatui-core ^0.1).

## 2. The contract (patchbay-core)

### 2.1 Manifest, registry, factory

```rust
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Footprint { pub w: u8, pub h: u8 }
impl Footprint {
    pub const fn new(w: u8, h: u8) -> Self { Self { w, h } }
    pub const TILE: Self = Self::new(1, 1);  pub const WIDE: Self = Self::new(2, 1);
    pub const PANEL: Self = Self::new(4, 2); pub const HERO: Self = Self::new(6, 3);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Procfs, Hwmon, Cpufreq, Rapl,            // /proc, /sys/class/hwmon, cpufreq, energy_uj readable
    Nvml,                                    // libnvidia-ml.so.1 dlopens and Nvml::init() succeeds
    I2cNvidia, AstralExporter, AstralCsv,    // /dev/i2c-N (NVIDIA) openable / :9942 answers / csv fresh
    PwRecord, PipeWireSocket,                // pw-record on PATH / $XDG_RUNTIME_DIR/pipewire-0
    DbusSession,                             // $DBUS_SESSION_BUS_ADDRESS connects
    PingSocket, NetRaw,                      // SOCK_DGRAM ICMP allowed / CAP_NET_RAW (per-process bw)
    TrueColor, VteGlyphs, Mouse,             // COLORTERM=truecolor, VTE_VERSION>=7800, mouse enabled
}
pub struct CapSet(BitSet);   // has(), missing(&[Capability]) -> Vec<Capability>, reason(cap) -> &str

pub struct KeyHint { pub key: &'static str, pub action: &'static str }
pub struct Tier { pub name: &'static str, pub min: Size }   // richest first; min = inner cells

pub struct Manifest {
    pub kind: &'static str,                  // "cpu" — the value of `kind =` in layout.toml
    pub name: &'static str,                  // "CPU & processes (htop)"
    pub summary: &'static str,
    pub version: &'static str,               // env!("CARGO_PKG_VERSION")
    pub contract: u32,                       // must equal CONTRACT_VERSION at registration
    pub footprints: &'static [Footprint],    // picker + `s` cycle; free resize still allowed
    pub default_footprint: Footprint,
    pub requires: &'static [Capability],     // all missing → instance built in Unavailable mode
    pub optional: &'static [Capability],     // missing → feature-level degradation
    pub services: &'static [&'static str],   // ServiceDef ids the factory will ask for
    pub example_options: &'static str,       // TOML, printed by `patchbay component info <kind>`
    pub keys: &'static [KeyHint],
}

pub struct ComponentDef {
    pub manifest: &'static Manifest,
    pub build: fn(&mut BuildCtx<'_>) -> Result<Box<dyn Component>, BuildError>,
}

pub struct Registry { components: BTreeMap<&'static str, ComponentDef>, services: BTreeMap<&'static str, ServiceDef> }
impl Registry {
    pub fn new() -> Self;
    pub fn component(&mut self, def: ComponentDef) -> &mut Self;   // panics on kind clash / contract mismatch
    pub fn service(&mut self, def: ServiceDef) -> &mut Self;
    pub fn get(&self, kind: &str) -> Option<&ComponentDef>;
    pub fn iter(&self) -> impl Iterator<Item = &ComponentDef>;
}
```

Every component crate exports `pub static DEF: ComponentDef` (and `pub fn register(r: &mut Registry)` which also registers the services it needs), so the cli crate is literally:

```rust
pub fn builtin_registry() -> Registry {
    let mut r = Registry::new();
    #[cfg(feature = "cpu")]   patchbay_comp_cpu::register(&mut r);
    #[cfg(feature = "gpu")]   patchbay_comp_gpu::register(&mut r);
    /* … */
    r
}
fn main() -> color_eyre::Result<()> { patchbay_app::run(builtin_registry(), Cli::parse()) }
```

A third party writes the same six lines plus `their_crate::register(&mut r)` and gets their own binary — this is the extension story (see §11).

### 2.2 Build and instance context

```rust
pub struct InstanceId(pub Arc<str>);       // "net-lan" — unique per layout, from [[components]].id

pub struct BuildCtx<'a> {
    pub instance: &'a InstanceId,
    pub options: &'a toml::Table,          // raw per-instance [components.options]
    pub services: &'a Services,
    pub caps: &'a CapSet,
    pub store: &'a TelemetryStore,
    pub bus: &'a Bus,
}
impl BuildCtx<'_> {
    /// `T: #[serde(default, deny_unknown_fields)]`; unknown keys are reported with the instance id.
    pub fn options<T: DeserializeOwned + Default>(&self) -> Result<T, BuildError>;
    /// Typed handle to a running service feed (started lazily by the shell on first request).
    pub fn feed<S: Service>(&self) -> Result<Arc<Feed<S>>, BuildError>;
}
```

### 2.3 The Component trait (object-safe; the shell draws the frame, the component draws the inside)

```rust
pub struct RenderCtx<'a> {
    pub inner: Rect,            // what you may draw into; outer border/title already drawn by the shell
    pub class: SizeClass,       // Tiny/Small/Medium/Large/Huge from `inner`
    pub footprint: Footprint,   // nominal, for information only — never size by it
    pub view: Option<&'a str>,  // placement-level hint, e.g. "procs"
    pub focused: bool, pub captured: bool, pub zoomed: bool, pub dense: bool,
    pub theme: &'a Theme, pub glyphs: &'a GlyphSet,
    pub now: Instant, pub frame: u64,
}
pub struct TickCtx<'a>  { pub now: Instant, pub visible: bool, pub store: &'a TelemetryStore, pub bus: &'a Bus }
pub struct InputCtx<'a> { pub inner: Rect, pub bus: &'a Bus, pub caps: &'a CapSet }

pub enum Redraw { No, Yes }
pub enum RedrawPolicy { OnChange, Animated { fps: u8 } }
pub enum Handled { No, Yes, Release }          // Release = component gives key capture back to the grid
#[derive(Clone, Debug, PartialEq)]
pub enum Health { Ok, Degraded(String), Unavailable(String) }

pub trait Component: Send {
    fn manifest(&self) -> &'static Manifest;
    fn title(&self, class: SizeClass) -> Cow<'_, str>;     // short forms for Tiny/Small
    fn tiers(&self) -> &'static [Tier];                    // shell picks first tier whose min fits inner
    fn tick(&mut self, ctx: &TickCtx<'_>) -> Redraw;       // pull from feeds/store; no I/O, no blocking
    fn render(&self, ctx: &RenderCtx<'_>, tier: usize, buf: &mut Buffer);
    fn redraw_policy(&self) -> RedrawPolicy { RedrawPolicy::OnChange }
    fn health(&self) -> Health { Health::Ok }
    fn on_key(&mut self, key: KeyEvent, ctx: &InputCtx<'_>) -> Handled { Handled::No }
    fn on_mouse(&mut self, ev: MouseEvent, local: Position, ctx: &InputCtx<'_>) -> Handled { Handled::No }
    fn on_visibility(&mut self, visible: bool) {}
    fn keymap(&self) -> &'static [KeyHint] { self.manifest().keys }
}
```

Rules enforced by the shell and testkit: `render` takes `&self` (so zoom, duplicate placements and snapshot tests never disturb state); `render` and `tick` run on the render thread with a per-frame budget; a component that panics is caught per frame (`catch_unwind` around `render`), marked `Unavailable("panicked: …")` and drawn as a chip — one bad tile cannot take the dashboard down.

### 2.4 Services and feeds (the data layer)

```rust
pub trait Snapshot: Serialize + DeserializeOwned + Send + Sync + 'static {}

pub trait Service: 'static {
    const ID: &'static str;                 // "nvml"
    type Snap: Snapshot;
    type Cmd: Send + 'static;               // () for read-only services
    /// Scalars to keep as history; runs on publish AND on replay so store history is identical.
    fn metrics(snap: &Self::Snap, sink: &mut dyn MetricSink);
}

pub struct Feed<S: Service> {
    latest: ArcSwapOption<S::Snap>, generation: AtomicU64,
    health: RwLock<Health>, cmd: Mutex<Option<Sender<S::Cmd>>>, interest: AtomicUsize,
}
impl<S: Service> Feed<S> {
    pub fn latest(&self) -> Option<Arc<S::Snap>>;
    pub fn generation(&self) -> u64;                          // cheap "did anything change" check in tick()
    pub fn health(&self) -> Health;
    pub fn send(&self, cmd: S::Cmd) -> Result<(), FeedClosed>; // e.g. MediaCmd::PlayPause
    pub fn acquire(&self) -> Interest;                        // RAII: visible components hold one
    pub fn interested(&self) -> bool;                         // service thread throttles/pauses when false
    // producer side (service threads):
    pub fn publish(&self, snap: S::Snap, store: &TelemetryStore, wake: &Wake);
    pub fn set_health(&self, h: Health);
}

pub struct ServiceCtx { pub caps: CapSet, pub store: Arc<TelemetryStore>, pub wake: Wake,
                        pub config: toml::Table /* [services.<id>] */, pub recorder: Option<RecorderHandle> }
pub struct ServiceDef {
    pub id: &'static str,
    pub requires: &'static [Capability],
    pub start:  fn(&ServiceCtx) -> Result<Arc<dyn AnyFeed>, ServiceError>,     // real hardware
    pub demo:   fn(&ServiceCtx, u64 /*seed*/) -> Arc<dyn AnyFeed>,             // deterministic synthetic
    pub replay: fn(&ServiceCtx, &Path, f32 /*speed*/) -> Result<Arc<dyn AnyFeed>, ServiceError>,
}
pub struct Services { feeds: HashMap<&'static str, Arc<dyn AnyFeed>> }
impl Services { pub fn feed<S: Service>(&self) -> Option<Arc<Feed<S>>>; }

/// Helper for poll-style services: owns a std thread, calls `sample()` at `interval()`,
/// halves the rate when nobody is interested, converts Err into Health::Degraded with backoff.
pub trait Sampler: Send + 'static {
    type Svc: Service<Cmd = ()>;
    fn sample(&mut self, now: Instant) -> Result<<Self::Svc as Service>::Snap, SourceError>;
    fn interval(&self, interested: bool) -> Duration;
}
pub fn spawn_sampler<P: Sampler>(p: P, ctx: &ServiceCtx) -> Arc<Feed<P::Svc>>;
```

Push-style services (audio, MPRIS) own their thread and call `feed.publish` directly. Because `demo`/`replay` are part of `ServiceDef`, **components never know whether data is live, synthetic or replayed** — that is what makes `--demo`, replay tests and CI without hardware free.

### 2.5 Telemetry store

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub struct MetricKey(pub &'static str); // "nvml.util", "procs.cpu.total"
#[derive(Clone, Copy)] pub struct Sample { pub at: Instant, pub v: f64 }
pub enum Agg { Last, Max, Mean }

pub struct Series { buf: VecDeque<Sample>, cap: usize, max_seen: f64 }
impl Series {
    pub fn push(&mut self, at: Instant, v: f64);
    pub fn last(&self) -> Option<Sample>;
    pub fn window(&self, d: Duration) -> impl Iterator<Item = &Sample>;
    /// Tick-rate independent: one bucket per column over the last `d` — what sparklines/charts consume.
    pub fn resample(&self, d: Duration, cols: u16, agg: Agg) -> Vec<Option<f64>>;
    pub fn max_in(&self, d: Duration) -> Option<f64>;
}
pub struct TelemetryStore { inner: RwLock<HashMap<MetricKey, Series>>, default_cap: usize /* 600 */ }
impl TelemetryStore {
    pub fn push(&self, key: MetricKey, at: Instant, v: f64);
    pub fn with<R>(&self, key: MetricKey, f: impl FnOnce(&Series) -> R) -> Option<R>;
}
pub trait MetricSink { fn push(&mut self, key: MetricKey, v: f64); }
```

Capacity is bounded per series (config `store.history = "10m"` → cap = duration/interval, clamped to 4096). The store holds scalars only; detail (process rows, per-pin arrays, art bitmaps) stays in the latest snapshot.

### 2.6 Bus (cross-cutting events to the shell)

```rust
pub struct AppAlert { pub source: InstanceId, pub severity: Severity, pub title: String, pub detail: String, pub since: Instant }
pub enum Severity { Info, Warning, Critical }
pub struct Bus { tx: Sender<Msg> }
impl Bus { pub fn raise(&self, a: AppAlert); pub fn clear(&self, source: &InstanceId, title: &str); pub fn toast(&self, text: impl Into<String>, sev: Severity); }
```

Data sharing between components does **not** go through the bus — it goes through shared feeds (the media component reads the audio feed for its mini-spectrum; gpu and pins both read `nvml`). The bus carries only alerts, toasts and wake-ups.

## 3. Layout engine, size classes, grid

Fixed unit grid per page: `columns = 12` (Home-Assistant/Grafana-style), `rows = 6` default (`"auto"` derives rows from terminal aspect with `cell_aspect = 0.5`). Placements are `{ id, at = [x, y], size = [w, h] }` in units; the engine is a pure integer function — no solver, deterministic, invertible for mouse hit-testing:

```rust
pub struct GridSpec { pub columns: u8, pub rows: Rows, pub gap: u8, pub borders: BorderMode, pub cell_aspect: f32, pub min_terminal: Size }
pub enum Rows { Fixed(u8), Auto }
pub enum BorderMode { Each, Shared, None }      // Shared = gap 0 + 1-cell overlap + Block::merge_borders(Exact)
pub struct Placement { pub id: InstanceId, pub at: [u8; 2], pub size: [u8; 2], pub view: Option<String>, pub priority: u8 }
pub struct Cell { pub id: InstanceId, pub outer: Rect, pub inner: Rect, pub class: SizeClass, pub tier: Option<usize>, pub starved: bool }
pub enum SolveMode { Grid, Dense, Stack, TooSmall }
pub struct Solved { pub mode: SolveMode, pub cells: Vec<Cell>, pub col_starts: Vec<u16>, pub row_starts: Vec<u16> }

pub fn tracks(len: u16, n: u8, gap: u16) -> Vec<(u16, u16)>;     // widths differ by ≤1, sum exact
pub fn solve(spec: &GridSpec, page: &Page, body: Rect, tiers: &dyn Fn(&InstanceId) -> &'static [Tier]) -> Solved;
pub fn hit(s: &Solved, pos: Position) -> Option<&Cell>;
pub fn unit_at(s: &Solved, body: Rect, pos: Position) -> Option<(u8, u8)>;
// pure edit ops (proptest: no overlap after any op sequence)
pub fn move_by(page: &Page, id: &InstanceId, dx: i8, dy: i8, spec: &GridSpec) -> Result<Page, EditError>;
pub fn resize_by(page: &Page, id: &InstanceId, dw: i8, dh: i8, spec: &GridSpec) -> Result<Page, EditError>;
pub fn swap(page: &Page, a: &InstanceId, b: &InstanceId) -> Result<Page, EditError>;
pub fn insert_first_fit(page: &Page, id: InstanceId, fp: Footprint, spec: &GridSpec) -> Result<Page, EditError>;
```

`SizeClass::of(inner)` buckets width `<12/<24/<48/<96` and height `<3/<6/<12/<24`, class = min of both. On a 250×70 Ptyxis with 12×6 units a 1x1 is ≈18×9 inner (Small), 2x1 Medium, 4x2 Large, 6x3/zoom Huge. Components pick detail through their own `tiers()` (min sizes), the class only steers titles and glyph density. Degradation ladder: configured → dense (gap 0, shared borders, short titles) → starved cell renders a chip `▪ cpu` → below `min_terminal` (80×24) the page switches to a priority-ordered stack with scrolling.

## 4. Theme

Components never see a `Color` literal or a glyph string; they ask for roles, gradients and named glyphs:

```rust
pub enum Role { Bg, Surface, Panel, Border, BorderFocused, Title, Text, TextMuted, TextGhost,
                AccentPrimary, AccentSecondary, AccentTertiary, Ok, Warn, Crit, Info, SelectionFg, SelectionBg, Cursor }
pub enum GradientId { Load, Temp, Power, Mem, NetRx, NetTx, Audio, Title }
pub enum ColorMode { TrueColor, Ansi256, Ansi16, Mono }
pub struct Gradient { lut: [Color; 64] }             // Oklab-interpolated, pre-downsampled for ColorMode
impl Gradient { pub fn at(&self, t: f32) -> Color; pub fn shade_at(&self, t: f32) -> &'static str /* ░▒▓█ for Mono */ }
pub struct GlyphSet { pub bar: &'static [&'static str], pub spark: &'static [&'static str], pub marker: Marker,
                      pub ok: &'static str, pub warn: &'static str, pub crit: &'static str, pub info: &'static str,
                      pub up: &'static str, pub down: &'static str, pub play: &'static str, pub pause: &'static str, /* … */ }
pub struct EffectHooks { pub startup: Option<EffectSpec>, pub theme_swap: Option<EffectSpec>, pub focus: Option<EffectSpec>,
                         pub alert: Option<EffectSpec>, pub ambient: AmbientSpec, pub budget_ms: u8 }
pub struct Theme { meta: Meta, colors: [Color; 19], gradients: EnumMap<GradientId, Gradient>, pub glyphs: GlyphSet,
                   pub borders: Borders, pub title: TitleSpec, pub flourish: Flourish, pub effects: EffectHooks, pub mode: ColorMode }
impl Theme {
    pub fn load(file: &ThemeFile, parent: Option<&ThemeFile>, mode: ColorMode) -> Result<Self, ThemeError>; // contrast-checked
    pub fn color(&self, r: Role) -> Color;  pub fn style(&self, r: Role) -> Style;
    pub fn gradient(&self, g: GradientId) -> &Gradient;
    pub fn severity(&self, s: Severity) -> (Style, &str);           // colour + glyph, BOLD|REVERSED in Mono
    pub fn block<'a>(&self, title: Line<'a>, focused: bool) -> Block<'a>;   // used by the shell only
    pub fn for_component(&self, kind: &str) -> Arc<Theme>;          // pre-merged [components.<kind>] overrides
}
```

`EffectSpec` is plain data (`{ kind, duration_ms, … }`); only `patchbay-app` maps it onto tachyonfx (`fx::sweep_in`, `fx::fade_from`, `fx::hsl_shift` ping-pong, `effect_fn` scanlines), scoped to the affected cell, bounded ≤600 ms for event effects, ambient off by default even in retrowave, with a `budget_ms` watchdog that disables ambient effects when the moving average exceeds it. `ColorMode` is resolved once (CLI > config > `NO_COLOR` > `COLORTERM` > `TERM`); under `NO_COLOR` the `mono` theme is loaded because crossterm 0.29 emits no colour SGR at all in that case.

Theme file (excerpt, `themes/retrowave.toml`):

```toml
[meta]     name = "retrowave"  schema = 1  variant = "dark"  inherits = "base-dark"
[palette]  indigo = "#0b0324"  violet = "#7a3fb5"  pink = "#ff2975"  cyan = "#00f0ff"  purple = "#b967ff"
           orange = "#ff8b39"  mint = "#05ffa1"  sun = "#fede5d"  red = "#fe4450"  snow = "#efe9ff"  dusk = "#8a7fb0"
[colors]   bg = "$indigo"  surface = "#1a0b3d"  panel = "#241b2f"  border = "$violet"  border_focused = "$pink"
           title = "$snow"  text = "$snow"  text_muted = "$dusk"  text_ghost = "#3d2a63"  cursor = "$pink"
[colors.accent]    primary = "$pink"  secondary = "$cyan"  tertiary = "$purple"
[colors.severity]  ok = "$mint"  warn = "$sun"  crit = "$red"  info = "$cyan"
[colors.selection] fg = "#ffffff"  bg = "#3d1a63"
[gradients] load = ["$cyan", "$purple", "$pink", "$orange"]  temp = ["$cyan", "$purple", "$pink", "$red"]
            audio = ["$cyan", "$pink", "$sun"]  title = ["#f6f0ff", "#c8b8ff", "$pink", "$violet"]
[glyphs]   set = "unicode"  nerd = false  bar = "nine_levels"  chart_marker = "octant_if_vte"   # braille fallback
[borders]  set = "double"  focused_set = "thick"  merge = "exact"
[title]    style = "gradient"  case = "upper"  bold = true
[flourish] grid_floor = true  sun = true  big_clock = { pixel = "quadrant" }  marquee = true
[effects]  enabled = true  budget_ms = 4
           startup = { kind = "sweep_in", motion = "left_to_right", duration_ms = 600 }
           alert   = { kind = "hsl_pulse", lightness = 25, period_ms = 900, target = "crit_fg" }
           ambient = { crt_scanlines = false, crt_flicker = false }
[components.audio] gradients.audio = ["$cyan", "$purple", "$pink"]
```

`modern.toml` is the same schema with Catppuccin Mocha values, `borders.set = "rounded"`, `title.style = "plain"`, every `flourish`/`effects` key false. A `terminal.toml` maps roles onto the 16 ANSI colours so it follows whatever Ptyxis palette is active. Themes are hot-reloaded (§7).

## 5. Data flow and threading

All blocking work is off the render thread; there is no tokio in core, app or cli. A service may run a private runtime inside its own thread (the MPRIS service uses `zbus`'s `blocking-api`; if reverse DNS ever lands in the net service it gets a private single-thread tokio inside that thread) — the runtime choice is an implementation detail hidden behind `Feed`.

```
input thread ──── event::read() ────────────────┐
service threads ─ Feed::publish → store.push ── wake ┤ mpsc::Receiver<Msg>       render thread
watcher thread ── notify(config/themes dir) ────┤   Msg::{Input, Wake(id), Alert,   owns Terminal, App,
recorder thread ─ JSONL writer ◄── publish taps ┘        Toast, Reload(kind), Quit}  pages, effects (!Send)
```

Frame loop (`patchbay-app::loop`): deadline = last + 1/fps (`fps = 30` default, `60` opt-in). Between deadlines it `recv_timeout`s messages: input is dispatched immediately (global → captured component → grid); `Wake` marks the owning cells dirty. At the deadline it draws only if `dirty || effects.running() || any visible component is Animated` — a static page costs nothing. Draw = `solve()` (cached per (page, body)) → for each cell: shell draws frame (theme block, title, health chip, focus) → `component.render(ctx, tier, buf)` under `catch_unwind` → per-cell effects → overlay (alert banner, toasts, help, edit ghosts) → global effect. A frame-time/changed-cell overlay (`F12`) exists from arc 1 so VTE throughput is measured, not guessed.

| Thread / service | Cadence | Notes |
|---|---|---|
| render | 30 fps target (60 opt-in), draws only when dirty/animated | budget 4 ms/frame at 250×70 |
| input | blocked in `event::read()` | sole reader; process exit reaps it |
| procs | 1500 ms (htop default), configurable; `smaps_rollup`/`io` only when a column needs them | full scan ~30–60 ms on torch, off-thread |
| nvml | fast tier 250 ms (util/temp/power/clocks/pstate/throttle), slow tier 1 s (memory, fans, processes, PCIe counters, 20 ms power samples), static once | never `pcie_throughput` (21 ms blocking) |
| pins | 500 ms while any pins/gpu cell is interested, 1000 ms otherwise, never stops (alarm overlay) | source auto: exporter → i2c → csv |
| net | counters 1 s (250 ms sub-tick when a sparkline is Animated), link attrs 2 s, probes 1 Hz, conn table 2 s only while interested | ICMP DGRAM, TCP-connect fallback |
| audio | pw-record chunks every ~10.6 ms; FFT (N=2048, Hann, 64 log bands 30 Hz–16 kHz, L/R) per chunk, publish ≤60 Hz; DSP paused and child killed after 30 s without interest | `node.passive=true`, supervisor with 250 ms→5 s backoff |
| mpris | property streams (push), Position poll 1 Hz while Playing, art decode on a worker | players discovered via NameOwnerChanged arg0ns |
| hwmon | 1 s | keyed by chip name + device path |
| watcher | debounced 250 ms | themes/ and config dir, NonRecursive |

## 6. How each component plugs in

Every row below is one component crate + the service(s) it consumes; footprints are the ones the manifest advertises (free resize is still allowed between min and max).

**cpu** (`patchbay-comp-cpu` ← `svc-procs`, optional `svc-hwmon` for Tccd temps, `Cpufreq`). Manifest: requires `Procfs`; footprints 1x1, 2x1, 4x2, 6x3, 12x6. Tiers: `big-number` (CPU % + sparkline from `procs.cpu.total`), `meters` (htop stacked `StackedBar` widget — nice/user/kernel/virt in htop colours through `Role`s — mem/swap bars, tasks/load/uptime), `cores` (2 CCD blocks × 8 cores × SMT pair from sysfs `die_id`, PSI row), `table` (top-N process table: PID USER CPU% MEM% RES S TIME+ Command, PID-keyed selection/tags), `full` (htop parity: screens, tree, search/filter, F-key bar, kill/renice/affinity/ioprio modals; `readonly` flag + kill confirmation). Keys are htop's verbatim once captured. Snapshot `ProcSnapshot { cpus: Vec<CpuBreakdown>, mem: MemInfo, load, psi, tasks, procs: Vec<ProcRow> }`; metrics `procs.cpu.total`, `procs.cpu.N`, `procs.mem.used`, `procs.load1`.

**gpu** (`patchbay-comp-gpu` ← `svc-nvml`, `svc-procs` for CPU%/RSS of GPU processes, optional `svc-pins`). Requires `Nvml`; optional `I2cNvidia|AstralExporter` (Power sub-panel gains pin bars). Tiers: `badge` (util % + temp), `gauges` (GPU/VRAM/MEMCTL + clocks/W/°C/fan + throttle chip), `header` (nvtop header parity: PCIe gen@width RX/TX from byte counters, ENC/DEC auto-hide, 3 fans %/RPM, 20 ms power trace), `charts` (10-min ring charts: util, VRAM, temp, power, clocks, effective-load; GPU-Z spec column from the const `SPECS` table keyed by PCI id 0x2B85…), `full` (+ process table PID USER TYPE GPU% ENC DEC GPU-MEM CPU% HOST-MEM Command, F6 sort, F9 signal). `GpuSnapshot` carries `Option<T>` per field; `NotSupported` fields are probed once and never polled again.

**pins** (`patchbay-comp-pins` ← `svc-pins`). Requires any of `I2cNvidia|AstralExporter|AstralCsv` (manifest lists all three as optional and the service picks: exporter → i2c → csv → Unavailable, re-probed every 10 s). The service links astral-watch as a pinned git rev (`default-features = false`, never `tui`/`safety`), runs `read_reading` on its own thread, evaluates `alert::evaluate` + `Lifecycle` (policy from `astral_watch::config::load(None)`) and raises `AppAlert`s on `Condition` edges (Overload/Disconnected/Imbalance → Critical banner; ImbalanceAdvisory → Warning toast; TelemetryLost → Info chip). Tiers: `watts-badge`, `mini-bars` (six eighth-block bars), `bars` (peak caps, 9.2 A limit line, balance gauge, totals), `trend` (+ watts sparkline + alert log), `full` (tui.rs parity: device header from the shared `nvml` feed + sysfs PCIe link, Braille trend chart, scrollable log, pause/reset/rate keys). Constants (`AMPS_CEILING 10`, `HISTORY 300`, 7.82/9.2 A colour bands, balance WARN 1.33/ALARM 1.5) are reused verbatim.

**net** (`patchbay-comp-net` ← `svc-net`). Requires `Procfs`; optional `PingSocket` (ICMP probes, else TCP-connect), `NetRaw` (Tier 1 per-process bandwidth via a future `patchbay-netd` helper), `DbusSession` (resolved DNS, NetworkManager Wi-Fi fallback). Options: `interfaces = ["en*","wl*","wg*"]`, `hide = ["veth*","br-*","docker*","virbr*"]`, `probes`, `rdns = false`, `public_ip = false`. Tiers: `rates` (↓/↑ for the default-route interface), `sparks` (rx/tx sparklines from `net.<if>.rx_bps` resampled), `table` (interfaces + drops/errs + probe strip), `conns` (top connections with own-PID attribution, uid otherwise), `full` (sortable connection table, interface detail pane, probe pane with jitter/loss). State keyed by (name, ifindex); `saturating_sub` everywhere.

**audio** (`patchbay-comp-audio` ← `svc-audio`). Requires `PwRecord` + `PipeWireSocket`. The service publishes `AudioFrame { bands_l: [f32; 64], bands_r: [f32; 64], wave: [f32; 512], rms: [f32; 2], peak: [f32; 2], rate: u32, sink: Arc<str>, seq: u64 }` — bands are instance-agnostic (64 log bands, dBFS-normalised with configurable floor/tilt); each component instance resamples to its bar count and applies its own ballistics (`preset = "winamp"` gravity + accelerating peak caps, `"cava"` gravity/integral/monstercat). `RedrawPolicy::Animated { fps: 60 }` while visible. Tiers: `vu` (stereo VU/peak pair), `mini` (8–10 thin bars), `scope` (Canvas octant on VTE, braille elsewhere), `spectrum` (mirrored stereo ⌊(w+1)/3⌋ thick bars, gradient per row from `GradientId::Audio`, `▔` peaks, sink name), `full` (spectrum + scope + VU + LUFS via `ebur128`). Options: `sink = "auto"|<node.name>`, `fft = 2048|4096`, `range = [30, 16000]`, `bars = "auto"|N`.

**media** (`patchbay-comp-media` ← `svc-mpris`, optional `svc-audio` for the 19-band Winamp vis and kHz/stereo fields). Requires `DbusSession`. Service: hand-rolled zbus `PlayerProxyBlocking`/`MediaPlayer2ProxyBlocking` with `Position` marked `emits_changed_signal = "false"`, per-player thread, supervisor picks Playing > most-recent > alphabetical, `MediaCmd::{PlayPause, Next, Prev, SeekRel, SeekAbs, Volume, Raise, Select}` via `Feed::send`. Track identity = hash(title|artist|album|url) (Firefox's trackid is constant); `length = None` ⇒ "stream mode" with a local elapsed clock. Art: `file://`/`https://`(ureq, 5 s, 8 MB cap)/`data:` → `image` decode on a worker → one pre-encoded `ratatui-image` `Protocol` per (art, tier); `Picker::halfblocks()` on VTE (Sixel/Kitty verified absent in Ptyxis), `from_query_stdio()` only behind `art = "auto"` and only before the input thread starts. Tiers: `status` (▶/‖/■ glyphs from `GlyphSet`, 1-row marquee, posbar), `shade` (Winamp shade mode: marquee + time + mini vis), `main` (7-segment time via `tui-big-text` Quadrant, marquee at 220 ms/char with `  ***  `, kbps/kHz/stereo from the PipeWire node, 19-band vis, posbar, volume, transport row, shuffle/repeat greyed when the player lacks the props), `main+art`, `full` (main + EQ (weights the visualiser bands only) + playlist built from a local Metadata-transition history). Player list, glyph sets and marquee separator are theme/option driven.

**sensors** (`patchbay-comp-sensors` ← `svc-hwmon`, optional `svc-nvml` so GPU temp/fan/power appear in the same list without duplication). Requires `Hwmon`; optional `Rapl` (rendered "needs udev rule" when `energy_uj` is 0400). `SensorReading { key, chip, label, kind, value, unit, warn, crit }` with nvme sentinel `_max > 1e6` filtered, hwmon keyed by `name@devpath`. Tiers: `hottest` (single worst-margin reading), `strip` (chips as chips), `table` (grouped by chip with warn/crit bars), `full` (+ per-sensor sparklines, PSI, cpufreq per CCD, `amd_x3d_mode`).

**clock** (`patchbay-comp-clock`, no service) is the documented template: 40 lines, `tui-big-text`, shows every trait method once.

## 7. Configuration

`~/.config/patchbay/config.toml` (behaviour + layout), `~/.config/patchbay/themes/*.toml`. Layering: built-in defaults ← file ← `PATCHBAY_*` env ← CLI (hand-rolled; figment would drag toml 0.8 next to toml 1.1). Validation after parse: unique ids/hotkeys, `kind` registered, options accepted by the factory, placements inside the grid, pairwise overlap (reports both ids with `toml::de::Error::span()` positions), footprint outside `manifest.footprints` → warning only. `patchbay config check|default|explain` and `patchbay component list|info <kind>` expose all of it.

```toml
schema = 1
theme = "retrowave"           # or "modern", "phosphor-green", "terminal", "mono"
fps = 30                      # 60 opt-in
mouse = true
[grid]        columns = 12  rows = 6  gap = 1  borders = "each"  min_terminal = { cols = 80, rows = 24 }
[store]       history = "10m"
[record]      dir = "~/.local/share/patchbay/recordings"      # `r` toggles recording
[services.pins]  source = "auto"  exporter = "127.0.0.1:9942"  csv = "~/tmp/gpu-pins.csv"  interval_ms = 500
[services.audio] sink = "auto"  latency = 512  fft = 2048

[[components]]  id = "cpu"    kind = "cpu"    options = { refresh_ms = 1500, hide_kernel_threads = true }
[[components]]  id = "gpu"    kind = "gpu"    options = { power_panel = true }
[[components]]  id = "pins"   kind = "pins"
[[components]]  id = "lan"    kind = "net"    options = { interfaces = ["eno1", "wl*"], probes = ["gateway", "1.1.1.1", "8.8.8.8"] }
[[components]]  id = "viz"    kind = "audio"  options = { preset = "winamp", bars = "auto" }
[[components]]  id = "amp"    kind = "media"  options = { players = ["firefox", "spotify"], art = "halfblocks" }
[[components]]  id = "temps"  kind = "sensors"

[[pages]]  name = "Overview"  hotkey = "1"
place = [
  { id = "cpu",   at = [0, 0], size = [6, 3] },  { id = "gpu",   at = [6, 0], size = [6, 3] },
  { id = "pins",  at = [0, 3], size = [4, 2] },  { id = "lan",   at = [4, 3], size = [4, 2] },
  { id = "viz",   at = [8, 3], size = [4, 2] },  { id = "amp",   at = [0, 5], size = [4, 1] },
  { id = "temps", at = [4, 5], size = [8, 1] },
]
[[pages]]  name = "Audio"  hotkey = "2"
place = [
  { id = "amp", at = [0, 0], size = [6, 3] },  { id = "viz", at = [6, 0], size = [6, 3] },
  { id = "cpu", at = [0, 3], size = [12, 3], view = "procs" },
]
```

Hot reload: `notify` watches the config directory (editors save by rename), 250 ms debounce, parse+validate on the watcher thread, then `Msg::Reload`. The shell diffs instances by `(kind, options)` and keeps unchanged ones (history survives), rebuilds the rest, swaps `Arc<Config>`/`Arc<Theme>` and fires the `theme_swap` effect; on error the old config stays and a toast shows `file:line:col`. Edit-mode saves go through `toml_edit::DocumentMut` (comments preserved), atomic temp+rename, re-parse verification, and a content hash so the watcher ignores self-writes.

## 8. Keyboard, mouse, edit mode

Global (never captured): `Ctrl-q` quit, `?` help, `F12` perf overlay. Grid mode: `1–9` pages, `[`/`]` prev/next, `Tab`/`Shift-Tab` reading-order focus, `h j k l`/arrows spatial focus (projection overlap, min edge distance), `Enter` capture keys into the focused component (its `keymap()` replaces the status bar), `Esc`/`Handled::Release` returns, `z` zoom toggle, `d` dense toggle, `t` cycle themes, `p` pause sampling, `r` record toggle, `e` edit mode, `q` quit (not while captured). Captured components keep their native muscle memory (htop's `F5`/`/`/`\`/`k`, nvtop's `F6`/`F9`, astral-watch's space/`+`/`-`/`1-5`).

Mouse (SGR, opt-out; Shift-drag keeps native selection in VTE): click focuses, double-click zooms, wheel goes to the hovered component with local coordinates, click on the title chip cycles footprint when in edit mode.

Edit mode is a state machine over the pure page ops: `H J K L` move one unit, `Ctrl-hjkl` resize (Shift shrinks), `s` cycle `manifest.footprints`, `S`+dir swap with neighbour, `a` picker (fuzzy list of registered kinds + existing instances; new instance → `insert_first_fit` with `default_footprint`), `x` remove placement (instance kept if placed elsewhere), `u`/`Ctrl-r` undo/redo (page snapshots), `w` save, `Esc` leave (prompt if dirty). Collisions are rejected with a red ghost; the dotted unit grid and `(x,y) w×h` readout are drawn in the overlay layer. Drag moves, dragging the bottom-right corner resizes.

## 9. Error handling and degraded modes

- **Startup probe** (`patchbay_app::probe(&Registry) -> CapSet`, ≤200 ms total, each probe on a thread with a timeout: file/socket existence, `Nvml::init()` on a thread, one `pw-record --version`, D-Bus connect, DGRAM ICMP socket creation). `patchbay doctor` prints the table with reasons.
- **Missing required capability** → the instance is still placed; `build` is skipped and the shell installs a `Placeholder` component (`Health::Unavailable("NVML: libnvidia-ml.so.1 not found")`) whose tiers show the reason and the fix (`apt install …`, `udev rule …`, `usermod -aG i2c`). Nothing else changes.
- **Runtime loss** → services set `Health::Degraded`/`Unavailable` with backoff-retried init (`GpuLost`, `LibRmVersionMismatch` → "driver/library mismatch — reboot", pw-record exit → respawn 250 ms→5 s, MPRIS player vanished → next player, i2c `TelemetryLost` → Lifecycle freeze). Components render stale data dimmed with a `STALE 12s` badge derived from `Feed::generation()` age; they never see `Err`.
- **No GPU**: gpu tile + pins Power sub-panel degrade; pins alone still works via i2c/exporter. **No i2c**: pins → exporter → CSV → placeholder. **No audio**: audio and the media vis degrade; media transport still works. **No D-Bus**: media placeholder; net loses resolved/NM lookups only.
- **Component panic** → caught per frame, chip, log line, counter in the perf overlay. **Stderr** is `dup2`'d to `$XDG_STATE_HOME/patchbay/patchbay.log` before the alternate screen (astral-watch's library `eprintln!`s would otherwise scribble on the UI); `tracing` goes to the same file and to an optional log page.
- Mouse capture and focus events are undone in a chained panic hook exactly as astral-watch does; `color_eyre` installed before `ratatui::run`.

## 10. Testing strategy

- **Unit**: layout `tracks()` sums/monotonic (proptest), edit ops never overlap (proptest), theme contrast gate + 256/16 downsampling tables, DSP (bin mapping, ballistics against known inputs), htop/nvtop formatting helpers (`Row_printKBytes`/time formats ported with their tables), astral-watch parser for exporter text and CSV.
- **Snapshot** (`patchbay-testkit`): `snapshot_matrix!(component, footprints = [1x1, 2x1, 4x2, 6x3, full], themes = ["modern", "retrowave", "mono"])` renders each into `Buffer`/`TestBackend` at the real cell sizes for 120×40 and 250×70 terminals and `insta::assert_snapshot!`s the text; colour assertions via `buffer.cell((x,y)).fg` for role mapping. Every component also runs `assert_never_panics(component, 0..=12 × 0..=6)`.
- **Replay**: `recordings/torch-2026-08-30/*.jsonl` (60 s, committed, ~1 MB) drive `ServiceDef::replay` in tests; snapshots are taken at fixed offsets so a rendering change is a reviewed diff, not a surprise. `patchbay record` produces new fixtures; `--replay <dir> --speed 4` plays them in the real UI for demos and bug reports.
- **Demo mode**: `patchbay --demo [--seed N]` starts every service's `demo` feed (seeded, deterministic, plausible: 32 cores with a game-like load, a 5090 at 400 W, a 1.5× idle imbalance, a 48 kHz synthetic mix, a fake Firefox player). CI runs `patchbay --demo --headless --size 250x70 --frames 90 --screenshot out.txt` (TestBackend) and diffs it — the same command generates README screenshots.
- **Integration**: config fixtures for every error path (`config check` exit codes), hot-reload of a bad file keeps the old one, capability-missing builds produce placeholders.
- **CI matrix**: `--no-default-features`, each component feature alone, `--all-features`; all tests run without hardware because nothing touches devices unless `start` is called.

## 11. Third-party components: static now, adapters later (honest assessment)

- **Static registration (chosen)**: a component is a crate depending on `patchbay-core`; the author assembles their own binary with `patchbay_app::run(registry, cli)`. Cost: a rebuild. Benefit: zero ABI risk, full type safety, works on musl, no toolchain lock. Also `[patch]`-friendly for local forks.
- **cdylib plugins (`abi_stable 0.11` / `stabby 72`)**: would require `#[repr(C)]` mirrors of `Buffer`, `Cell`, `Style`, `Rect`, `KeyEvent` (~2 kLOC + conversions each frame), identical rustc for host and plugin, and gives no crash isolation. Not worth it for a personal dashboard; revisit only if several external authors appear.
- **WASM (`extism 1.30` / `wasmtime 48`)**: feasible because the contract already separates data (serde snapshots via `Feed`) from drawing; the bridge would be a `DrawList` (`Vec<(Rect, Style, CompactString)>`) host function set plus snapshot bytes per tick. Cost: +50 MB of build deps, +10–20 s cold compile, a second rendering abstraction, and host functions for every feed. Benefit: sandboxing and any language. Planned only as an optional `patchbay-wasm-host` crate after 1.0, and only if wanted.
- **External-process components** (`kind = "exec"`, sampler/wtfutil style: run a command, read JSON lines or a draw list on stdout): ~300 lines, any language, no ABI, and it covers most "I just want my script in a tile" needs. Scheduled as its own small arc before any WASM work.

The contract is versioned by `CONTRACT_VERSION` and `patchbay-core` semver; manifests are plain data so a sidecar `manifest.toml` could describe an out-of-process component with no code changes.

## 12. Performance budget (250×70 Ptyxis, measured with the F12 overlay from arc 1)

- Render: ≤4 ms per drawn frame in release (ratatui diff + widgets + effects), ≤40 % of cells changed per frame outside the audio cell; audio cell ≤120×30 cells of per-cell RGB at 60 fps (~3.6 k cells/frame). Static pages: 0 draws.
- Services combined: <2 % of one core at the default cadences with all components visible (procs ~50 ms/1.5 s, nvml <5 ms/s, pins 4–33 ms/0.5 s, audio DSP <0.3 ms per 10.6 ms chunk); pw-record adds one passive PipeWire stream and must not lower the graph quantum (`latency 512`).
- Memory: store ≤4096 samples × ≤400 series ≈ 30 MB worst case; audio ring 8192 frames/channel; art cache 8 entries ≤256 px.
- Startup: probe ≤200 ms, first frame <300 ms; NVML init (~10 ms) and pw-record spawn happen on their service threads.
- Config/theme reload <50 ms; edit-mode ops O(placements).

## 13. Packaging and CI

Mirrors astral-watch: `ci.yml` = fmt, `clippy --all-targets -D warnings` (workspace), `cargo test --workspace`, `cargo doc -D warnings`, release build `--locked`, MSRV job on 1.88 (`cargo check --workspace --locked`), feature-matrix job, `cargo deny check` (licences + `cargo tree -d` bans), `cargo audit`, headless demo screenshot job. `release.yml` builds gnu + musl tarballs (nvml dlopens at runtime, zbus/procfs/realfft are pure Rust — musl is fine; pins needs the git dep so release builds run with network), nfpm deb/rpm with a `Recommends: pipewire-bin, nvidia-utils` line, an AUR `PKGBUILD` and a Nix flake in a later arc. Because astral-watch is a git dependency, crates.io publication is deferred until astral-watch 0.8.0 is published; `cargo install --git` is the documented path. `CHANGELOG.md` (Keep a Changelog), one minor version per arc, implement → adversarial review → fix → report → user approves commit.

## 14. Adding a new component (step list)

1. `cargo new --lib crates/components/foo --name patchbay-comp-foo`; depend on `patchbay-core`, `ratatui-core`, `ratatui-widgets` and the service crate(s) you read.
2. If the data does not exist yet, add `crates/services/foo` implementing `Service` (+ `Snapshot` type with serde, `metrics()`), a `Sampler` or push thread, and the three constructors for `ServiceDef { start, demo, replay }`.
3. Define `static MANIFEST: Manifest` (kind, footprints, requires/optional capabilities, services, `example_options`, keys) and `Options` with `#[serde(default, deny_unknown_fields)]`.
4. Implement `Component`: `tiers()` (richest first with min sizes), `tick()` (read `feed.latest()` when `generation()` changed, push derived state), `render()` per tier using only `ctx.theme`/`ctx.glyphs`, optional keys/mouse.
5. `pub static DEF: ComponentDef` and `pub fn register(r: &mut Registry)` that registers the service defs and the component.
6. Add `foo = ["dep:patchbay-comp-foo"]` to the cli's features and one `#[cfg(feature = "foo")]` line in `builtin_registry()`; add the crate to the CI feature matrix.
7. Tests: `snapshot_matrix!` across footprints × themes, `assert_never_panics`, a replay test if a recording exists, and a demo generator that looks plausible.
8. Docs: one section in `docs/COMPONENTS.md` generated from `patchbay component info foo`; CHANGELOG entry.

Nothing else changes: the layout engine, theme, edit mode, hot reload, recorder, demo mode and alert overlay all pick the new kind up from the registry.

## Key decisions

- **Contract shape**: One object-safe `Component` trait (tick/render(&self)/on_key/on_mouse/tiers/health) plus a static `Manifest` and a `ComponentDef { manifest, build }` registered in a `Registry`; the shell draws the frame, components only draw their inner Rect. — Object safety keeps `Box<dyn Component>` and future adapters (exec/WASM) possible; `render(&self)` makes zoom, duplicates and snapshot tests side-effect free; the shell-owned frame guarantees theme consistency and lets one bad component be caught per frame without touching neighbours.
- **Data layer**: All telemetry comes from `Service`s that publish serde-able `Snapshot`s through `Feed<S>` (arc-swap latest + generation + health + command channel + interest count) and push scalars into a shared `TelemetryStore`; every `ServiceDef` ships `start`, `demo` and `replay` constructors. — Components never touch devices or threads, two components can share one source (nvml for gpu+pins, audio for viz+media, procs for cpu+gpu), and recording, replay, demo mode and hardware-free CI are implemented once at the service seam instead of per component.
- **Threading**: std threads + `std::sync::mpsc` into a render thread that owns Terminal, App and tachyonfx effects; one thread per service; no tokio in core/app/cli (a service may run a private runtime inside its own thread). — Matches the ratatui FAQ and astral-watch's existing style; every collector here is blocking (procfs, NVML, i2c, pw-record pipe, FFT); tachyonfx `Effect` is !Send; zbus has a blocking API so MPRIS needs no shared runtime.
- **Layout model**: Fixed 12×6 unit grid per page with `{at, size}` placements solved by a pure integer track function; ratatui `Layout` only inside components and chrome; pure edit ops (`move_by/resize_by/swap/insert_first_fit`) with a snapshot undo stack and `toml_edit` write-back. — Every dashboard product (Grafana, Home Assistant, wtfutil, sampler) converged on unit grids; integer solving is deterministic, invertible for mouse hit-testing, proptest-able, and turns add/remove/rearrange into arithmetic; 12 columns make 1x1/2x1/4x2/6x3 map to ~20-cell units on the user's 250×70 terminal.
- **Size classes**: Components declare `tiers()` with minimum inner sizes (richest first) and the shell picks the first that fits the real inner Rect; `SizeClass` (Tiny..Huge from inner width/height buckets) only steers titles and glyph density; nominal footprint is informational. — A 6x3 on a laptop is smaller than a 4x2 on the workstation; Android/WidgetKit/HA all size from the host-reported real size; min-size tiers are explicit, testable per component and degrade predictably to a placeholder chip.
- **Theme system**: Semantic roles + Oklab gradient LUTs + named glyph sets + border/title/flourish specs + declarative effect hooks in TOML with `$palette` indirection, `inherits` and per-component overrides; colour mode resolved once (CLI > config > NO_COLOR > COLORTERM); mono theme under NO_COLOR; effects are data mapped to tachyonfx only in the app crate. — Lets one render path produce retrowave and modern from files alone; crossterm 0.29 emits no colour SGR under NO_COLOR so a mono theme with modifiers/shade ramps is required; keeping tachyonfx out of core means component crates stay light and effect budgets are enforced centrally.
- **Capabilities and degradation**: A startup `CapSet` probe (≤200 ms, per-probe timeout) plus `requires`/`optional` capability lists in each manifest; missing required → placeholder tile with reason and fix; runtime losses become `Health::Degraded/Unavailable` with backoff, never errors; stderr is dup2'd to a log file before entering the alternate screen. — This machine already lacks lm-sensors, RAPL access, Sixel, kitty keyboard protocol, dev headers and the astral-watch service; the dashboard must be useful with any subset present, and astral-watch's library eprintln!s would otherwise corrupt the UI.
- **Extensibility strategy**: Static Cargo-feature registration now; the app is a library (`patchbay_app::run(Registry, Cli)`) so third parties build their own binary; an `exec` (external process, JSON-lines/draw-list) component before any dynamic loading; cdylib/abi_stable rejected; WASM (extism/wasmtime) deferred to an optional post-1.0 host crate. — ABI-stable plugins need repr(C) mirrors of ratatui types and a locked toolchain for no isolation gain; WASM costs ~50 MB of deps and a second rendering abstraction; both are only justified by an ecosystem that does not exist yet, while the serde-snapshot + draw-only contract keeps both doors open cheaply.
- **Audio capture**: Default backend is a supervised `pw-record --format f32 --rate 48000 --channels 2 --raw --latency 512 --target auto -P '{ stream.capture.sink = true, node.passive = true }' -` child with realfft DSP in-process publishing 64 instance-agnostic log bands; cpal/pipewire-rs/pulseaudio crates only behind features. — Verified working on torch with no headers; cpal cannot build without libasound2-dev; passive streams never wake idle DACs; publishing generic bands lets the visualiser and the Winamp component apply their own bar counts and ballistics without a second FFT.
- **astral-watch integration**: Pin astral-watch as a git `rev` (tag once 0.8.0 exists) with `default-features = false`, consume only its library API (i2c/decode/alert/lifecycle/config/cards), auto-select exporter → i2c → CSV, run `Lifecycle` locally in every mode and raise app alerts on `Condition` edges; never enable its `tui` feature. — The crate is not on crates.io and HEAD's API differs from v0.7.0; the exporter is authoritative when the service runs, direct i2c is safe alongside the root logger (kernel per-adapter lock makes SMBus block reads atomic), and enabling `tui` would duplicate ratatui 0.29 next to 0.30.
- **Testing and demo**: insta snapshots via a `snapshot_matrix!` (footprints × themes × terminal sizes) in `patchbay-testkit`, proptests for layout/edit ops, committed 60 s JSONL recordings for replay tests, and a deterministic seeded `--demo` mode whose headless screenshot runs in CI and produces README images. — TestBackend's Display makes golden screens cheap; replay fixtures turn rendering changes into reviewed diffs; demo mode is the only way CI, a laptop without the GPU, and screenshots share one code path — and it exercises the same service seam users rely on.
- **Toolchain and dependencies**: Workspace `rust-version = 1.88` (ratatui 0.30.2 floor), avoid sysinfo (MSRV 1.95, no drop counters/class breakdown) in favour of procfs 0.18; core depends on ratatui-core/ratatui-widgets, app on the ratatui facade with crossterm 0.29; `cargo deny` + `cargo tree -d` in CI; hand-rolled 256-colour mapping instead of LGPL ansi_colours; toml 1.1 + toml_edit, no figment. — Keeps the MSRV job honest and the dependency graph single-versioned, mirrors ratatui's guidance for widget libraries, and avoids licence and duplicate-crate surprises the research flagged.

## Proposed first arc

Arc 1 = v0.1.0 "the contract proves itself": (1) workspace scaffold with the crate layout above, workspace-pinned deps (ratatui 0.30.2, crossterm 0.29, procfs 0.18, toml 1.1, palette 0.7.7, arc-swap, insta, proptest), rust-version 1.88, deny.toml, Makefile, CHANGELOG, CI (fmt/clippy -D warnings/test/doc -D warnings/MSRV/feature-matrix/deny/audit); (2) patchbay-core complete for the contract surface used this arc: Footprint/SizeClass/Tier, Capability + CapSet, Manifest/ComponentDef/Registry, Service/Feed/ServiceDef/Services/spawn_sampler, TelemetryStore with resample, Bus, layout engine (tracks/solve/hit + pure edit ops with proptests), Theme loader with roles/gradients/glyphs/borders and the contrast gate, embedded modern.toml + retrowave.toml + mono.toml (no effects yet); (3) patchbay-app: ratatui::run shell, input thread, frame loop with dirty gating and 30/60 fps, pages/hotkeys, Tab and hjkl focus, Enter/Esc capture, z zoom, t theme cycle, d dense, capability probe + placeholder tiles, stderr redirect, chained panic hook, F12 frame-time/changed-cells overlay, config load + validation + notify hot reload (no edit mode yet); (4) patchbay (cli): builtin_registry with features, `run`, `--demo`, `--headless --screenshot`, `config check|default`, `component list|info`, `doctor`; (5) two components: `clock` (template) and `cpu` with svc-procs (KernelStats/Meminfo/LoadAverage/PSI sampler at 1.5 s, real + demo + replay) rendering tiers big-number, meters (StackedBar in htop segment colours via roles), cores (CCD blocks from sysfs die_id) — the process table is arc 2; (6) patchbay-testkit with snapshot_matrix!, assert_never_panics, synthetic clock, and a committed 60 s procs recording from torch; (7) docs/ARCHITECTURE.md and docs/ADDING-A-COMPONENT.md, README with the demo screenshot. Exit criteria: `cargo test --workspace` passes with no hardware, `patchbay --demo` shows both themes at 250×70 in Ptyxis with the perf overlay reporting <4 ms/frame, and the adversarial review has been applied before the user approves the commit.
