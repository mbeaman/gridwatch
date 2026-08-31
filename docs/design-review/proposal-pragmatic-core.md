<!-- Architecture proposal (superseded by docs/ARCHITECTURE.md). Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Proposal: opstui

_Angle: pragmatic-core_

**Philosophy.** opstui is one Rust package (a library with a thin binary) whose only real abstractions are the three that demonstrably pay rent: a `Component` trait so tiles can be arranged on a grid and rendered at several footprints, a `Source` trait so every sampler thread gets identical scheduling, recording and replay for free, and a `Theme` value so no component ever names a colour or a glyph. Everything else is plain modules, std threads, `mpsc` channels and `Arc<Mutex<Arc<T>>>` latest-value slots, exactly the shape astral-watch already uses, so each session can add a component or a theme without touching the core. The bar for arc 1 is a screen you would screenshot: three real components (htop-style CPU, nvtop-style GPU, astral-watch pins) on a 24-column grid in a truecolor retrowave theme, with snapshot tests, replay fixtures, a demo mode and the same CI discipline as astral-watch, and a clear slot for the other four components to drop into.

# opstui — architecture (pragmatic core-first)

## 1. Shape of the repo

One package, `opstui` (repo `github.com/mbeaman/opstui`), edition 2024, `rust-version = "1.88"` (ratatui 0.30.2's declared MSRV; sysinfo is deliberately not used because it would force 1.95). Library + binary like astral-watch: the lib is what tests, doc tests and the render matrix exercise; `main.rs` is clap plus `App::run`. No workspace: a second crate would only buy compile-time isolation the components do not need, and it would double the CI matrix.

```
opstui/
├── Cargo.toml                    # [lib] opstui + [[bin]] opstui, rust-version 1.88
├── deny.toml                     # cargo-deny: licences (MIT/Apache/BSD/Zlib/Unicode only), bans tokio, cpal, mpris
├── README.md  CHANGELOG.md  LICENSE(MIT)  THIRD_PARTY.md
├── .github/workflows/{ci.yml,release.yml}
├── packaging/{nfpm.yaml,opstui.udev-rapl.rules}       # arc 5
├── assets/
│   ├── config.default.toml       # include_str!, printed by `opstui config default`
│   └── themes/{retrowave,modern,phosphor-green,mono}.toml   # include_str!, user dir overrides by name
├── fixtures/                     # recorded Source snapshots (JSONL) for replay tests and --demo
│   ├── cpu-game-1.5s.jsonl  gpu-game-250ms.jsonl  pins-idle.jsonl  pins-overload.jsonl
├── src/
│   ├── lib.rs                    # `pub mod` list only
│   ├── main.rs                   # clap Cli -> stderr redirect -> App::run
│   ├── cli.rs                    # --config --theme --page --fps --color --demo --replay DIR --record DIR --stats --no-mouse
│   ├── app.rs                    # App: render loop, Msg, pages/focus/zoom, alert board, frame stats
│   ├── input.rs                  # the one thread that calls crossterm event::read()
│   ├── config.rs                 # serde structs, defaults, validation (spans), mtime watcher thread
│   ├── grid.rs                   # pure layout engine: tracks/solve/hit + edit ops on Page
│   ├── theme/
│   │   ├── mod.rs                # Theme, Role, GradientId, Glyphs, Borders, ColorMode
│   │   ├── file.rs               # ThemeFile (serde) + `$palette` + inherits resolution
│   │   ├── color.rs              # nearest_256, nearest_16, wcag contrast, oklab mix
│   │   └── effects.rs            # arc 4: tachyonfx hooks behind `effects` cargo feature
│   ├── source/
│   │   ├── mod.rs                # Source trait, Sample<T>, SourceStatus, Ctl, Feeds registry
│   │   ├── spawn.rs              # spawn_source(): the sampler thread loop (+ record)
│   │   ├── replay.rs             # ReplaySource<T> from JSONL; DemoSource<T> from a closure
│   ├── widgets/                  # theme-aware building blocks, each `impl Widget for &X`
│   │   ├── stacked_bar.rs        # htop 4-segment meter (Bar/Text/Graph/Led modes)
│   │   ├── vbars.rs              # vertical eighth-block bars w/ gradient + peak caps (pins, audio)
│   │   ├── big.rs                # 3x3 / quadrant big digits (tui-big-text wrapper)
│   │   ├── halfblock.rs          # RGB image -> ▀ cells (album art, sun/grid flourish)
│   │   ├── chip.rs               # `▪ cpu` placeholder / status chips
│   │   └── table.rs              # sortable/selectable table with column drop order
│   ├── components/
│   │   ├── mod.rs                # Component trait, RenderCtx, SizeClass, Footprint, registry()
│   │   ├── clock.rs              # 90-line template component (arc 1)
│   │   ├── cpu/{mod.rs,sample.rs,procs.rs,render.rs}      # htop            (arc 1)
│   │   ├── gpu/{mod.rs,nvml.rs,specs.rs,smi.rs,render.rs}  # nvtop + GPU-Z   (arc 1)
│   │   ├── pins/{mod.rs,source.rs,render.rs}               # astral-watch    (arc 1)
│   │   ├── net/{mod.rs,dev.rs,link.rs,probe.rs,conns.rs,render.rs}          (arc 2)
│   │   ├── sensors/{mod.rs,hwmon.rs,psi.rs,render.rs}                       (arc 2)
│   │   ├── audio/{mod.rs,capture.rs,dsp.rs,render.rs}                       (arc 3)
│   │   └── player/{mod.rs,mpris.rs,art.rs,render.rs}       # winamp/MPRIS   (arc 3)
│   ├── overlay.rs                # alert banner + toasts, help popup, `--stats` overlay
│   └── util/{fmt.rs,ring.rs,stderr.rs}   # human units, fixed ring buffer, dup2 of fd 2 to a log file
└── tests/
    ├── render_matrix.rs          # every component × footprint × theme, plus 0x0..12x5 no-panic sweep
    ├── replay.rs                 # fixtures -> feeds -> assertions on rendered text
    └── snapshots/                # insta .snap files
```

Cargo features: `default = ["gpu","pins"]`; `gpu = ["dep:nvml-wrapper"]`, `pins = ["dep:astral-watch"]`, `audio = ["dep:realfft"]`, `player = ["dep:zbus","dep:image"]`, `effects = ["dep:tachyonfx"]`. Everything builds on a driverless, header-less CI runner because nvml-wrapper dlopens, astral-watch only needs `/dev/i2c-*` at runtime, audio capture is a subprocess, zbus is pure Rust.

## 2. The three traits and the core types

### 2.1 Component (src/components/mod.rs)

```rust
use ratatui::{buffer::Buffer, layout::{Position, Rect, Size}, crossterm::event::{KeyEvent, MouseEvent}};

pub type ComponentId = String;                       // instance id from config, e.g. "net-lan"

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct Footprint { pub w: u8, pub h: u8 }         // grid units

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum SizeClass { Tiny, Small, Medium, Large, Huge }
impl SizeClass {
    /// From the *inner* rect (after borders), never from the nominal footprint.
    pub fn of(inner: Size) -> Self {
        let w = match inner.width  { 0..=11 => 0, 12..=23 => 1, 24..=47 => 2, 48..=95 => 3, _ => 4 };
        let h = match inner.height { 0..=2  => 0, 3..=5   => 1, 6..=11  => 2, 12..=23 => 3, _ => 4 };
        [Self::Tiny, Self::Small, Self::Medium, Self::Large, Self::Huge][w.min(h)]
    }
}

pub struct RenderCtx<'a> {
    pub area: Rect,            // inner area the component may paint
    pub class: SizeClass,
    pub focused: bool,
    pub zoomed: bool,
    pub theme: &'a Theme,
    pub now: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)] pub enum Handled { Yes, No }

pub struct AppAlert { pub source: ComponentId, pub severity: Severity, pub title: String, pub detail: String, pub resolved: bool }

pub trait Component {
    fn kind(&self) -> &'static str;
    fn title(&self, class: SizeClass) -> std::borrow::Cow<'_, str>;
    fn footprints(&self) -> &'static [Footprint];                 // picker + `s` cycle
    fn min_inner(&self) -> Size { Size::new(8, 2) }               // below this: placeholder chip
    fn fps(&self) -> u8 { 0 }                                     // 0 = redraw only when tick() reports change
    /// Pull from feeds, advance histories/animations. Returns true if the picture changed.
    fn tick(&mut self, now: Instant) -> bool;
    fn render(&self, ctx: &RenderCtx, buf: &mut Buffer);
    fn on_key(&mut self, _key: KeyEvent) -> Handled { Handled::No }
    fn on_mouse(&mut self, _ev: MouseEvent, _local: Position) -> Handled { Handled::No }
    fn keys(&self) -> &'static [(&'static str, &'static str)] { &[] }   // status-bar hints
    fn drain_alerts(&mut self) -> Vec<AppAlert> { Vec::new() }
    fn set_visible(&mut self, _visible: bool, _class: SizeClass) {}     // sources back off when hidden
    fn status(&self) -> SourceStatus;                                   // for the title badge
}

pub type Factory = fn(&toml::Table, &mut Feeds) -> anyhow::Result<Box<dyn Component>>;
pub fn registry() -> &'static [(&'static str, Factory)] { &[("clock", clock::new), ("cpu", cpu::new), ("gpu", gpu::new), ("pins", pins::new), /* arc 2+: */ ] }
```

`render(&self)` is deliberately by reference, matching ratatui 0.30's `impl Widget for &W` convention: the same component instance can be drawn zoomed, in a snapshot test, or twice on one page without touching its history. State that changes on user input lives in `Cell<..>`-free plain fields mutated only in `on_key`/`tick`.

### 2.2 Source, Sample, Feed (src/source/mod.rs)

```rust
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SourceStatus { Ok, Degraded(String), Unavailable(String), Starting }

pub struct Ctl { pub visible: AtomicBool, pub detail: AtomicU8, pub interval_ms: AtomicU32, stop: AtomicBool }

pub trait Source: Send + 'static {
    type Snap: Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static;
    fn name(&self) -> &'static str;                    // feed key and fixture file stem
    fn interval(&self) -> Duration;                    // default cadence; Ctl may override
    /// Blocking. Must not panic; failures become the status. Called on the sampler thread.
    fn poll(&mut self, ctl: &Ctl) -> Result<Self::Snap, SourceError>;
}
pub enum SourceError { Degraded(String), Unavailable(String) }   // Unavailable => exponential backoff up to 30 s

pub struct Sample<T> { pub seq: u64, pub at: Instant, pub wall: SystemTime, pub status: SourceStatus, pub data: Option<Arc<T>> }

#[derive(Clone)]
pub struct Feed<T> { slot: Arc<Mutex<Arc<Sample<T>>>>, ctl: Arc<Ctl> }
impl<T> Feed<T> {
    pub fn latest(&self) -> Arc<Sample<T>> { self.slot.lock().unwrap().clone() }   // one Arc clone, microseconds
    pub fn seq(&self) -> u64;
    pub fn ctl(&self) -> &Ctl { &self.ctl }
}

/// One sampler thread per *source kind*, shared by every component instance that asks for it.
pub struct Feeds { map: HashMap<&'static str, Box<dyn Any + Send + Sync>>, wake: mpsc::Sender<Msg>, mode: FeedMode }
pub enum FeedMode { Live, Replay { dir: PathBuf, speed: f32 }, Demo }
impl Feeds {
    pub fn get_or_spawn<S: Source>(&mut self, make: impl FnOnce() -> S, demo: fn(u64) -> S::Snap) -> Feed<S::Snap>;
    pub fn get<T: 'static>(&self, name: &str) -> Option<Feed<T>>;   // cross-component reads (player -> audio bars)
}
```

`spawn_source` (src/source/spawn.rs) is the only place that knows about threads:

```rust
pub fn spawn_source<S: Source>(mut src: S, feed: Feed<S::Snap>, wake: mpsc::Sender<Msg>, record: Option<Recorder>) -> JoinHandle<()> {
    thread::Builder::new().name(format!("src-{}", src.name())).spawn(move || {
        let ctl = feed.ctl.clone(); let mut seq = 0; let mut backoff = Duration::from_millis(250);
        loop {
            if ctl.stop.load(Relaxed) { return; }
            let started = Instant::now();
            let (status, data) = match src.poll(&ctl) {
                Ok(s) => { backoff = Duration::from_millis(250); (SourceStatus::Ok, Some(Arc::new(s))) }
                Err(SourceError::Degraded(m)) => (SourceStatus::Degraded(m), None),
                Err(SourceError::Unavailable(m)) => { backoff = (backoff * 2).min(Duration::from_secs(30)); (SourceStatus::Unavailable(m), None) }
            };
            seq += 1;
            if let Some(r) = &record { r.append(seq, started, &status, data.as_deref()); }   // JSONL line
            *feed.slot.lock().unwrap() = Arc::new(Sample { seq, at: started, wall: SystemTime::now(), status, data });
            let _ = wake.send(Msg::Wake);
            let base = Duration::from_millis(ctl.interval_ms.load(Relaxed) as u64);
            let period = if ctl.visible.load(Relaxed) { base } else { base.max(Duration::from_secs(1)) };
            sleep_interruptible(&ctl.stop, if matches!(status, SourceStatus::Unavailable(_)) { backoff } else { period.saturating_sub(started.elapsed()) });
        }
    }).expect("spawn sampler")
}
```

`ReplaySource<T>` implements `Source` by reading the JSONL fixture and honouring recorded deltas (÷ speed); `DemoSource<T>` wraps `fn(u64) -> T`. Because both go through the same `Feed`, components cannot tell live from replay from demo, which is what makes the snapshot tests and `--demo` screenshots honest.

### 2.3 Theme (src/theme/mod.rs)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role { Bg, Surface, Panel, Border, BorderFocused, Title, Text, TextMuted, TextGhost,
                AccentPrimary, AccentSecondary, AccentTertiary, Ok, Warn, Crit, Info, SelectionFg, SelectionBg, Cursor }
pub const ROLES: usize = 19;
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GradientId { Load, Temp, Power, Mem, NetRx, NetTx, Audio, Title }
#[derive(Clone, Copy)] pub enum ColorMode { TrueColor, Ansi256, Ansi16, Mono }
#[derive(Clone, Copy)] pub enum Severity { Ok, Info, Warn, Crit }

#[derive(Clone)] pub struct Gradient { lut: [Color; 64] }
impl Gradient { pub fn at(&self, t: f32) -> Color; pub fn shade_at(&self, t: f32) -> &'static str /* " ░▒▓█" for Mono */ }

#[derive(Clone)] pub struct Glyphs { pub bar_levels: &'static [&'static str; 9], pub sparkline: &'static [&'static str; 9],
    pub marker: ratatui::symbols::Marker, pub ok: &'static str, pub warn: &'static str, pub crit: &'static str, pub info: &'static str,
    pub arrow_up: &'static str, pub arrow_down: &'static str, pub peak: &'static str, pub limit: &'static str }
#[derive(Clone)] pub struct Borders { pub normal: ratatui::symbols::border::Set, pub focused: ratatui::symbols::border::Set, pub merge: Option<MergeStrategy> }
#[derive(Clone, Copy)] pub enum TitleStyle { Plain, Badge, Gradient, Bracketed }

#[derive(Clone)]
pub struct Theme { pub name: String, pub mode: ColorMode, colors: [Color; ROLES], gradients: [Gradient; 8],
                   pub glyphs: Glyphs, pub borders: Borders, pub title: TitleStyle, pub upper_titles: bool, pub dim_muted: bool }
impl Theme {
    pub fn load(file: &ThemeFile, parent: Option<&ThemeFile>, mode: ColorMode) -> Result<Self, ThemeError>;   // resolves $palette, downsamples, warns on WCAG < 4.5 (text) / 3.0 (graphics)
    pub fn builtin(name: &str) -> Option<&'static str>;          // include_str! of assets/themes
    pub fn color(&self, r: Role) -> Color { self.colors[r as usize] }
    pub fn style(&self, r: Role) -> Style { Style::new().fg(self.color(r)) }
    pub fn gradient(&self, g: GradientId) -> &Gradient { &self.gradients[g as usize] }
    pub fn severity(&self, s: Severity) -> (Style, &'static str);       // colour + glyph; Crit adds BOLD|REVERSED; Mono keeps modifiers only
    pub fn block(&self, title: &str, focused: bool, badge: Option<Line<'static>>) -> Block<'static>;   // borders, panel bg, title_top (gradient/badge/plain), badge top-right
    pub fn title_line(&self, text: &str) -> Line<'static>;             // Gradient => one Span per char via gradient(Title)
}
```

Downsampling (`nearest_256`, `nearest_16`) and WCAG contrast are ~60 lines in `color.rs`; Oklab interpolation uses `palette 0.7.7` (pure Rust) because hand-rolling Oklab is where hobby gradient code goes wrong. Under `NO_COLOR` or `--color=never`, `mode = Mono`: every role becomes `Color::Reset` and meaning is carried by `REVERSED`/`BOLD`/`DIM` and shade ramps, exactly astral-watch's `Theme { color: bool }` idea generalised.

### 2.4 Layout engine (src/grid.rs) — pure functions, no solver

```rust
#[derive(Clone, serde::Deserialize)] pub struct GridSpec { pub columns: u8, pub rows: Rows, pub gap: u8, pub borders: BorderMode, pub cell_aspect: f32, pub min_terminal: Size }
#[derive(Clone, Copy, serde::Deserialize)] pub enum Rows { Fixed(u8), Auto }
#[derive(Clone, Copy, PartialEq, Eq, serde::Deserialize)] pub enum BorderMode { Each, Shared, None }
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Placement { pub id: ComponentId, pub at: [u8; 2], pub size: [u8; 2], #[serde(default = "prio")] pub priority: u8 }
#[derive(Clone, serde::Deserialize)] pub struct Page { pub name: String, pub hotkey: Option<char>, pub rows: Option<u8>, pub place: Vec<Placement> }

pub struct Cell { pub id: ComponentId, pub outer: Rect, pub inner: Rect, pub class: SizeClass, pub starved: bool }
pub enum SolveMode { Grid, Dense, Stack, TooSmall { need: Size, have: Size } }
pub struct Solved { pub mode: SolveMode, pub cells: Vec<Cell>, pub col_starts: Vec<u16>, pub row_starts: Vec<u16> }

/// Track edges: widths differ by <=1, sum exact, monotonic (invertible for mouse).
pub fn tracks(len: u16, n: u8, gap: u16) -> Vec<(u16, u16)>;
pub fn solve(spec: &GridSpec, page: &Page, body: Rect, min_inner: &dyn Fn(&str) -> Size) -> Solved;
pub fn hit(s: &Solved, pos: Position) -> Option<&Cell>;
pub fn unit_at(s: &Solved, pos: Position) -> Option<(u8, u8)>;
pub fn neighbour(s: &Solved, from: &ComponentId, dir: Dir) -> Option<ComponentId>;   // spatial focus: overlap on the orthogonal axis, min edge distance

// Edit-mode ops (arc 4) are pure and property-testable: no overlap, in bounds, >= min footprint.
pub enum EditError { Overlap(ComponentId), OutOfBounds, BelowMin }
pub fn move_by(page: &Page, id: &str, dx: i8, dy: i8, spec: &GridSpec) -> Result<Page, EditError>;
pub fn resize_by(page: &Page, id: &str, dw: i8, dh: i8, spec: &GridSpec, fps: &[Footprint]) -> Result<Page, EditError>;
pub fn swap(page: &Page, a: &str, b: &str) -> Result<Page, EditError>;
pub fn insert_first_fit(page: &Page, id: ComponentId, fp: Footprint, spec: &GridSpec) -> Result<Page, EditError>;
pub fn remove(page: &Page, id: &str) -> Page;
```

Default grid: 24 columns × 6 rows. On the 250×70 workstation terminal a unit is ~10×11 cells; the 2:1 footprint family (2×1, 4×2, 6×3) reads as roughly square, 1×1 as a tall tile. `Rows::Auto` = `clamp(round(H·columns·cell_aspect/W), 3, 12)`. Shared-border mode uses `gap 0`, extends non-last spans by one cell and `Block::merge_borders(MergeStrategy::Exact)`; themes that use rounded corners are forced to `Each`.

Degradation ladder per frame: configured → dense (gap 0, shared borders, `title(Small)`) → cells whose inner rect is below `min_inner()` draw a `chip` → below `min_terminal` (80×24) the page renders in **stack mode** (placements by priority desc, then reading order, `Layout::vertical` of `Constraint::Min(min_h)`) with a one-line notice; arc 1 ships the notice, arc 4 adds scrolling.

## 3. Threads, channels, tick rates

```
input thread  ── event::read() ──▶ mpsc<Msg> ──▶ RENDER THREAD (owns Terminal, App, all Box<dyn Component>, Theme)
src-cpu   (1.5 s) ─┐                                   ▲
src-gpu   (250 ms) ─┤ Feed<T> slots + Msg::Wake ───────┘
src-pins  (500 ms) ─┤
src-net   (1 s)    ─┤   config watcher (1 s mtime stat) ──▶ Msg::ConfigChanged / Msg::ThemeChanged
src-audio (reader) ─┘
```

```rust
pub enum Msg { Input(crossterm::event::Event), Wake, ConfigChanged(Result<Config, String>), ThemeChanged(Result<ThemeFile, String>) }

impl App {
    pub fn run(cli: Cli) -> color_eyre::Result<()> {
        // 1. stderr -> ~/.local/state/opstui/opstui.log via libc::dup2 (astral-watch's eprintln!s must not hit the alt screen)
        // 2. Config::load, Theme::load, Feeds::new(mode), build components from config via registry()
        // 3. spawn input thread (sole caller of event::read) and the config watcher
        // 4. ratatui::run(|terminal| app.main_loop(terminal, rx)); mouse capture enabled/disabled around it + chained panic hook
    }
    fn main_loop(&mut self, terminal: &mut DefaultTerminal, rx: mpsc::Receiver<Msg>) -> io::Result<()> {
        let mut next = Instant::now();
        while !self.quit {
            let fps = self.effective_fps();                       // max(component.fps() over visible cells), clamped to cfg.fps (30 default, 60 opt-in)
            let frame = Duration::from_micros(1_000_000 / fps.max(4) as u64);
            let now = Instant::now();
            let changed = self.tick(now);                          // every component.tick(); drains alerts; recomputes layout if size/page changed
            if changed || self.dirty || self.overlay.animating() {
                let t0 = Instant::now();
                terminal.draw(|f| self.draw(f))?;                 // paints Bg over the whole frame first, then chrome, cells, overlays
                self.stats.record(t0.elapsed(), terminal.backend()...);
                self.dirty = false;
            }
            next = (next + frame).max(now);
            loop { match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(Msg::Input(ev)) => self.on_event(ev),
                Ok(Msg::Wake) => self.dirty = true,
                Ok(Msg::ConfigChanged(r)) => self.apply_config(r),
                Ok(Msg::ThemeChanged(r)) => self.apply_theme(r),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => self.quit = true,
            } }
        }
        Ok(())
    }
}
```

Rates: cpu 1.5 s (htop default), gpu fast tier 250 ms with slow fields every 4th tick, pins 500 ms, net 1 s (250 ms optional for sparklines), sensors 1 s, mpris 1 s position poll while Playing, audio reader continuous (pw-record delivers 512 frames every 10.6 ms; DSP runs in `tick()` on the render thread at the frame rate because a 2048-point realfft is ~10 µs). Hidden components get `visible=false` → sources drop to ≥1 s; pins never stop (the alarm overlay depends on it).

## 4. How each component plugs in

Every component follows the same file split: `mod.rs` (struct, `new` factory, `Component` impl, `Options` serde struct), `sample.rs`/`source.rs` (a `Source` impl), `render.rs` (one `fn render_<class>` per size class, sharing widgets).

**cpu (htop parity)** — `CpuSource` on `procfs 0.18` with `default-features = false`: `KernelStats`, `Meminfo`, `LoadAverage`, PSI, per-core `cpufreq/scaling_cur_freq`, k10temp `Tccd1/2` by label, CCD map from `topology/die_id`. `CpuSnap { total: CpuBreakdown, cores: Vec<CpuBreakdown>, mem: MemMeter, swap: SwapMeter, load: [f32; 3], tasks: Tasks, uptime: Duration, freq_mhz: Vec<u32>, ccd_temp: [Option<f32>; 2], psi: Psi, procs: Option<Vec<ProcRow>> }` using htop's formulas verbatim (guest subtracted, `cached = Cached + SReclaimable − Shmem`, Irix-mode CPU%). `ctl.detail` (set from `set_visible(_, class)`) = 0 meters only, 1 + top-N procs, 2 + full columns; `smaps_rollup`/`io` only at level 2. Footprints `[1x1, 2x1, 4x2, 6x3, 12x6]`: Tiny = big CPU% + sparkline; Small = stacked CPU/mem/swap bars + load/tasks; Medium = two CCD blocks × 8 cores with SMT pairs, temps, MHz; Large = + top-N process table (`widgets::table` with drop order VIRT, SHR, PRI, NI, S, RES, TIME+, USER, PID, MEM%, CPU%); Huge = htop keybindings (`P M T N` sort, `/` search, `\` filter, `t` tree, `K H`), process actions (`F9` kill, `F7/F8` renice via `nix 0.31` + `libc::setpriority`) arrive in arc 5 behind `confirm_kill` and `readonly`.

**gpu (nvtop + GPU-Z)** — `GpuSource` owns `Nvml` and the `Device` inside its thread closure. Fast tier: `utilization_rates`, `temperature`, `power_usage`/`POWER_INSTANT` field, `clock_info` ×4, `performance_state`, `current_throttle_reasons`. Slow tier (1 s): `memory_info` (v2, `reserved` excluded), enc/dec, fans (`fan_speed`, `fan_speed_rpm`), `running_graphics/compute_processes`, `process_utilization_stats(now_us − tick_us)`, PCIe byte-counter fields 197/198 diffed (never `pcie_throughput`, which blocks 21 ms), `samples(Sampling::Power)` for the 20 ms power trace. Static once: name, uuid, arch, CC, cores, bus width, VBIOS, max clocks, limits, thresholds. `specs.rs` is a `const SPECS: &[GpuSpec]` table keyed by PCI device id (2B85, 2B87, 2C02, 2C05, 2F04, 2D04, 2D05, 2684, 2702, 2704, 2705) cross-checked against NVML `num_cores`/`memory_bus_width` at start. Per-field `NotSupported` latches to `None` and stops polling. `smi.rs` (nvidia-smi CSV, astral-watch's parser) is used only when `Nvml::init()` returns `LibloadingError`; `LibRmVersionMismatch` renders "driver/library mismatch — reboot". Footprints `[1x1, 2x1, 4x2, 6x3, 12x6]`: Tiny = util% + temp badge; Small = GPU/VRAM gauges + clocks/power/temp line; Medium = nvtop header parity (PCIe gen@width RX/TX, GPU/MEMCTL/VRAM/ENC/DEC bars, fans, power vs limit with 50 Hz sparkline); Large = + 10-minute charts (util, VRAM, temp, power, clocks, effective load) and the GPU-Z spec column; Huge = + process table (CPU%/RSS from the shared `cpu` feed's proc rows).

**pins (astral-watch)** — `astral-watch = { git = "https://github.com/mbeaman/astral-watch", rev = "dce7eee…", default-features = false }` with a `[patch]` to the sibling checkout for local work; never the `tui` or `safety` features. `PinsSource { bus: Option<u32>, pci: Option<String>, misses: u32, lifecycle: Lifecycle, cfg: astral_watch::config::Config }` mirrors `main.rs::run`: `detect_bus` every 5 s until `Found` (feeding `TelemetryLost`), `read_reading` at 500 ms, `evaluate`, `lifecycle.observe(now, &conds)`, `redetect_card` after `REDETECT_AFTER` misses. `PinsSnap { reading: Option<Reading>, live: bool, alerts: Vec<Alert>, events: Vec<Event>, active: Vec<Condition>, bus: Option<u32>, pci: Option<String> }`. Thresholds and alert policy come from `astral_watch::config::load(None)` so on-screen alarms agree with the service. The component keeps `HISTORY = 300` per-pin rings, peaks, watts ring and a 200-line log, and turns lifecycle `Event::{Raised, Repeated, Resolved}` into `AppAlert`s (Overload/Disconnected/Imbalance → Crit banner; ImbalanceAdvisory → Warn toast; TelemetryLost → Info chip). Footprints `[1x1, 2x1, 4x2, 6x3]`: Tiny = big total W + balance badge + alert glyph; Small = six eighth-block mini bars + `Σ W`; Medium = full-height `vbars` with peak caps, `┄` 9.2 A limit, values, balance gauge; Large = + watts sparkline + log + banner row (tui.rs parity). Exporter-scrape and CSV-tail modes are arc 5 (`[components.pins.options] source = "auto"`).

**net** (arc 2) — `NetSource`: `/proc/net/dev` (`procfs::net::dev_status`) + sysfs link attrs every 2 s; rates from `Instant` deltas with `saturating_sub`, state keyed by `(name, ifindex)`; default route from `/proc/net/route`; glob show/hide lists (`en* wl* wg*` vs `veth* br-* docker* virbr*`). `ProbeSource` (own thread, 1 Hz): `std::net::TcpStream::connect_timeout` to gateway:53, 1.1.1.1:443, 8.8.8.8:443 with min/avg/max/jitter/loss over 60 samples; DGRAM ICMP via `socket2` comes later (verified unprivileged here, but TCP-connect needs nothing). Connection table (`procfs::net::tcp/tcp6/udp` + `/proc/*/fd` inode map on the sampler thread, 2 s, level ≥1 only) is Large/Huge. Per-process bandwidth is a future helper binary; the header shows a capability badge.

**sensors** (arc 2) — hand-rolled hwmon walker keyed by `name` + device path (`temp*_input` with `_label/_max/_crit`, sentinel `_max > 1e6` dropped, chips with no inputs skipped), PSI, RAPL only if `energy_uj` is readable (else a "needs udev rule" chip). `SensorReading { key, chip, label, kind, value, warn, crit }`; the GPU feed's temp/fan/power are merged in at render time so nothing is polled twice.

**audio** (arc 3) — `capture.rs` spawns `pw-record --format f32 --rate 48000 --channels 2 --raw --latency 512 --target auto -P '{ stream.capture.sink = true, node.passive = true, node.name = "opstui audio" }' -`, a reader thread pushes f32 frames into `Arc<Mutex<Ring>>` (8192 frames/channel), supervisor respawns with 250 ms → 5 s backoff, >250 ms without data = silence (bars decay). `dsp.rs`: `realfft` N=2048 Hann, log-spaced bars (30 Hz–16 kHz), dBFS floor −65, +4 dB/oct tilt, attack/release EMA plus selectable `winamp` (falloff/16, accelerating peak) and `cava` (gravity + monstercat) smoothing; bar count = ⌊(w+1)/3⌋ thick or w thin. Renders with `widgets::vbars` (gradient `Audio`) and `Canvas` `Marker::Octant` scope (VTE draws octants natively). `fps() = 60` when visible. Sink picker reads `pw-dump` on demand.

**player / winamp** (arc 3) — `MprisSource` runs `zbus::blocking::Connection::session()` on its thread with two hand-rolled `#[proxy]` traits (Player with `Position` marked `emits_changed_signal = "false"`, root); discovery by `ListNames` prefix + `NameOwnerChanged` with `arg0ns`; commands arrive on an `mpsc::Receiver<PlayerCmd>` the source drains each loop. `NowPlaying { title, artist, album, length_us: Option<i64>, pos_us, pos_at: Instant, rate, status, can: Caps, art: Option<Arc<RgbImage>> }`; position interpolated locally; track change = hash of title|artist|album|url (Firefox's trackid is constant). Art: `image 0.25` (png/jpeg/webp only) decoded on the source thread, drawn by `widgets::halfblock` — no ratatui-image, no Sixel (VTE has none). Footprints: 1×1 status glyph + marquee, 2×1 shade mode (marquee + time + 8-bar mini spectrum from the `audio` feed), 4×2 the classic main window (big digits via `tui-big-text` Quadrant, marquee, kbps/kHz from pw-dump, 19-band spectrum, posbar, volume, transport row), 6×3 + art or EQ (EQ weights the visualizer bands), Huge + local "recently played" list.

## 5. Config and theme files

`~/.config/opstui/config.toml` (layout + instances), `~/.config/opstui/themes/<name>.toml` (overrides built-ins by name). Both are plain `serde` + `toml 1.1` with `deny_unknown_fields`; validation reports `file:line:col` via `toml::de::Error::span()` and semantic errors name both ids (overlap, unknown kind, out of bounds, unsupported footprint = warning).

```toml
schema = 1
theme = "retrowave"
fps = 30                      # 60 opt-in; audio component raises it while visible
mouse = true
[grid]
columns = 24
rows = 6                      # or "auto"
gap = 1
borders = "each"              # each | shared | none
min_terminal = { cols = 80, rows = 24 }

[[components]]
id = "cpu"
kind = "cpu"
[components.options]
interval_ms = 1500
hide_kernel_threads = true

[[components]]
id = "gpu"
kind = "gpu"
options = { fast_ms = 250, show_specs = true }

[[components]]
id = "pins"
kind = "pins"
options = { source = "auto", interval_ms = 500 }

[[components]]
id = "clock"
kind = "clock"

[[pages]]
name = "Overview"
hotkey = "1"
[[pages.place]]
id = "cpu"
at = [0, 0]
size = [12, 3]
priority = 100
[[pages.place]]
id = "gpu"
at = [12, 0]
size = [12, 3]
[[pages.place]]
id = "pins"
at = [0, 3]
size = [6, 3]
priority = 90
[[pages.place]]
id = "clock"
at = [6, 3]
size = [2, 1]
```

Theme file (retrowave excerpt; `modern.toml` is the same schema with `title.style = "plain"`, `borders.set = "rounded"`, no flourish):

```toml
[meta]
name = "retrowave"
variant = "dark"
inherits = "base-dark"
[palette]
indigo = "#0b0324"
pink = "#ff2975"
cyan = "#00f0ff"
purple = "#b967ff"
orange = "#ff8b39"
[colors]
bg = "$indigo"
panel = "#241b2f"
border = "#7a3fb5"
border_focused = "$pink"
text = "#efe9ff"
text_muted = "#8a7fb0"
[colors.accent]
primary = "$pink"
secondary = "$cyan"
tertiary = "$purple"
[colors.severity]
ok = "#05ffa1"
warn = "#fede5d"
crit = "#fe4450"
info = "$cyan"
[gradients]
load = ["$cyan", "$purple", "$pink", "$orange"]
title = ["#f6f0ff", "#c8b8ff", "$pink", "#7a3fb5"]
[glyphs]
set = "unicode"               # ascii | unicode; nerd never default
chart_marker = "braille"
[borders]
set = "double"
focused_set = "thick"
[title]
style = "gradient"
case = "upper"
```

Hot reload (arc 2): a watcher thread `stat()`s the config and active theme file once per second and sends `Msg::ConfigChanged`/`ThemeChanged` on mtime change; the app parses, validates, keeps instances whose `(kind, options)` are unchanged (history survives), rebuilds the rest, and keeps the old config on error with a toast. No `notify` dependency: one stat per second is free and immune to editor rename tricks.

## 6. Keyboard, mouse, edit mode

Two-level focus. Grid level: `Tab`/`Shift-Tab` reading order, `h j k l`/arrows spatial (`grid::neighbour`), `1–9` pages, `[ ]` prev/next page, `z`/`Enter` zoom focused tile (Esc back), `?` help, `t` cycle theme, `d` dense toggle, `p` pause all sources, `S` screenshot to `~/.local/state/opstui/shot-<ts>.txt` (buffer as text, handy for bug reports), `q`/`Ctrl-C` quit. Component level: `Enter` on a focused tile gives it the keyboard (status bar shows `component.keys()`), `Esc` returns; components use htop/nvtop bindings verbatim where they exist. Mouse (SGR, opt-out `mouse = false`): click focuses (`grid::hit`), double-click zooms, wheel goes to the tile under the cursor as a local `Position`, Shift-drag stays native VTE selection. Edit mode (arc 4, `e`): `HJKL` move one unit, `Ctrl-hjkl` resize, `s` cycle supported footprint, `x` remove, `a` picker (fuzzy list of registered kinds), `u`/`Ctrl-r` undo/redo (page snapshots), `w` save via `toml_edit` (comment-preserving, atomic temp+rename, re-parse check, self-write hash), collisions rejected with a red ghost; drag moves, corner drag resizes.

## 7. Error handling and degraded modes

Rules: `App::run` never fails because a component fails; a factory error (bad options) becomes a chip with the message; a source failure is a `SourceStatus`, rendered as a title badge (`▲ degraded`, `■ n/a`) plus the reason in Medium+ classes and in `?`. No `unwrap` outside tests; buffer writes go through `Buffer::cell_mut` (returns `Option`) or widgets that clip. Concrete cases: **no GPU** — `Nvml::init` `LibloadingError` → nvidia-smi tier if present, else `Unavailable("libnvidia-ml.so.1 not found")` with 30 s retry; `LibRmVersionMismatch` → "reboot after driver update"; a hot-unplugged device (`GpuLost`) → re-init with backoff. **no i2c** — `Detect::PermissionDenied` → "add yourself to the i2c group"; `NoBuses` → "no NVIDIA i2c adapters"; `NoTelemetry` → "waiting for telemetry (GPU idle?)" retried every 5 s while feeding `TelemetryLost` so no false all-clear; implausible readings are `TelemetryLost`, not errors. **no audio** — `pw-record` missing → "pipewire-bin not installed"; child exit → supervised respawn with backoff, stderr tail in the badge; passive stream on an idle sink → silence decay, not a hang. **no session bus / no players** — player tile shows "no MPRIS players" and keeps watching `NameOwnerChanged`. **terminal** — `NO_COLOR` → Mono theme; `COLORTERM` missing → 256 downsampling; width < 80 → stack mode. stderr is redirected to a log file at startup so third-party `eprintln!` never corrupts the alternate screen; panics chain `DisableMouseCapture` ahead of ratatui's restore hook, as in tui.rs.

## 8. Testing

1. **Unit tests** next to the code for every pure function: `grid::tracks` (sum exact, monotonic, ≤1 cell variance; cross-checked against `Layout::horizontal(vec![Fill(1); n]).spacing(Spacing::Overlap(1))`), `SizeClass::of`, edit ops (proptest: no overlaps after random op sequences), htop formulas against hand-computed `/proc/stat` deltas, memory formula, `nearest_256` on known hexes, WCAG on the shipped palettes, DSP (full-scale sine → 0 dBFS bin, bar mapping), MPRIS metadata decoding from recorded `a{sv}` maps, nvtop effective-load and PCIe counter deltas.
2. **Snapshot matrix** (`tests/render_matrix.rs`, `insta 1.48`): every registered component × every supported footprint × {retrowave, modern, mono} rendered from a `DemoSource` at a fixed tick into `TestBackend`; `assert_snapshot!(terminal.backend())` for glyphs plus explicit `buffer.cell((x,y)).fg` assertions for theme colours. A second sweep renders every component at every `Rect` from 0×0 to 12×5 and only asserts "no panic".
3. **Replay tests** (`tests/replay.rs`): `fixtures/*.jsonl` recorded on torch with `--record` (a game running, idle, a synthetic pin overload produced by editing a fixture) are fed through `ReplaySource` at speed 0 (instant); assertions check the rendered text ("OVERLOAD", "PWRCAP", process names) and the alert board. Fixtures are the regression net for parser changes.
4. **Demo mode** (`opstui --demo`): every source replaced by its `DemoSource`; used for screenshots in README/CI and for developing renderers on machines without the hardware. `--replay DIR` does the same with real data.
5. **Component-level integration tests**, ignored by default, run on torch: `cargo test -- --ignored` reads real NVML/i2c/pw-record for 1 s and checks frame counts.
CI (`ci.yml`): fmt, clippy `--all-targets --all-features -D warnings`, `cargo test --all-features`, `cargo doc -D warnings`, MSRV 1.88 `cargo check --locked --all-features`, `cargo deny check`, and a `--demo` smoke run under `script(1)` that writes a screenshot artifact.

## 9. Performance budget (250×70 terminal, 17,500 cells)

- Render thread: `tick()` for all components ≤ 1 ms; `draw` (widgets + diff) ≤ 4 ms at 30 fps, ≤ 8 ms at 60; `--stats` overlay shows frame time p50/p95, changed cells and bytes written so the VTE throughput question is answered in arc 1 on the real layout.
- Idle CPU of the whole process (no audio, 30 fps with dirty gating so most frames draw nothing) < 2 % of one core; with the audio tile at 60 fps < 8 %; RSS < 60 MB (NVML alone maps ~20 MB).
- Source threads: cpu scan < 60 ms at 1.5 s (measured < 10 ms per /proc file class), gpu fast tier < 50 µs, slow tier < 6 ms, pins 4–33 ms of kernel busy-poll per 500 ms, audio reader idle-waits on the pipe.
- Rules that keep it there: reuse `Style` values across runs, keep animated regions scoped (audio bars, sparklines), never allocate per cell in `render`, precomputed 64-entry gradient LUTs, `Layout` only inside components (thread-local cache), effects (arc 4) area-bounded with a `budget_ms` watchdog that disables ambient effects.

## 10. Packaging and CI

Mirrors astral-watch: `dtolnay/rust-toolchain` + `Swatinem/rust-cache`, CHANGELOG.md with one minor version per arc, `release.yml` on tags building `x86_64-unknown-linux-gnu` and `-musl` tarballs (musl is fine: NVML is dlopened, zbus/realfft are pure Rust, pw-record is a subprocess), `nfpm` deb/rpm, AUR PKGBUILD and a Nix flake in arc 5. Not published to crates.io until astral-watch is (git dependencies are rejected); `cargo install --git` is the documented path. `deny.toml` bans LGPL/NC licences (no `ansi_colours`), `tokio`, `cpal`, `mpris`, `pipewire`, `libpulse*` so an accidental transitive pull fails CI. Runtime requirements documented per feature: `libnvidia-ml.so.1` (gpu), `i2c` group (pins), `pipewire-bin` (audio), a session bus (player).

## 11. Adding a component (the checklist a future session follows)

1. `mkdir src/components/<kind>` with `mod.rs`, `source.rs`, `render.rs`; copy `clock.rs` as the skeleton.
2. Define `Options` (serde, `deny_unknown_fields`, `Default`) and `<Kind>Snap` (serde, `Clone`).
3. Implement `Source` for `<Kind>Source`: `poll` returns `Ok(snap)` or `Err(Degraded/Unavailable)`; never panic, never block longer than the interval.
4. Write `fn demo(tick: u64) -> <Kind>Snap` producing plausible moving data.
5. Implement `Component`: `footprints()`, `min_inner()`, `tick()` (read `feed.latest()`, compare `seq`, push rings), one `render_<class>` per class, `status()`.
6. Register the factory in `registry()`; the factory calls `feeds.get_or_spawn(|| <Kind>Source::new(&opts), demo)`.
7. Add the component to `assets/config.default.toml` and to the render matrix list; run `cargo insta review`.
8. Record a fixture on the real machine with `opstui --record fixtures/ --page 1` and add a replay assertion.
9. Document runtime requirements and degraded states in README; bump CHANGELOG.

## 12. Arc plan

- **Arc 1 (v0.1)**: core (sections 2–3, 5 without hot reload, 6 grid-level keys, 7, 8, 9), themes retrowave/modern/mono, components clock/cpu/gpu/pins, demo + replay + record, CI.
- **Arc 2 (v0.2)**: net + sensors, hot reload, phosphor and imported base16 themes, stack mode scrolling.
- **Arc 3 (v0.3)**: audio + player (winamp), 60 fps path measured.
- **Arc 4 (v0.4)**: edit mode + `toml_edit` save, tachyonfx effect hooks behind `effects`, flourishes.
- **Arc 5 (v0.5)**: htop process actions, nvtop process table, pins exporter/CSV modes, packaging (nfpm/AUR/Nix), RAPL udev rule, net helper design.

## Key decisions

- **Crate structure**: One package `opstui` with a library and a thin binary, plain modules per concern, no workspace, no plugin system; components are in-tree behind Cargo features (gpu, pins, audio, player, effects). — Matches astral-watch's proven lib+bin shape, keeps one CI matrix, and every reference tool that tried runtime plugins (gotop) abandoned them. Features let a driverless/header-less CI runner build everything while keeping optional deps out of the minimal build.
- **Abstractions**: Exactly three traits/types carry the design: `Component` (render at a footprint, handle keys), `Source` (blocking `poll` → typed snapshot) and `Theme` (roles, gradients, glyphs, borders). Everything else is functions and structs. — Each pays for itself: `Component` is what makes the grid rearrangeable; `Source` gives every sampler identical scheduling, back-off, pause-when-hidden, recording and replay from one 40-line spawner; `Theme` is what makes retrowave and modern share one render path. A data-store trait or an event-bus abstraction would be speculative.
- **Concurrency**: std threads only: one input thread blocked in `event::read()`, one sampler thread per source kind publishing into an `Arc<Mutex<Arc<Sample<T>>>>` latest-value slot plus an `mpsc` wake, and a render thread that owns the Terminal, App and components. No tokio. — Every collector is blocking (procfs, NVML, i2c ioctls, a pipe from pw-record); zbus has a blocking API; the ratatui FAQ says async only helps stdin. It is also the model the user already writes in tui.rs, so sessions stay fast and the code stays familiar.
- **Layout model**: Fixed unit grid (24 columns × 6 rows default, per-page override, optional auto rows) with integer placements, solved by a pure track function; ratatui `Layout` only inside components and for chrome. Size class is computed from the real inner Rect, never from the nominal footprint. — Grafana, Home Assistant, wtfutil and sampler all converged on unit grids; it makes move/resize/swap integer arithmetic, is invertible for mouse hit-testing, deterministic and unit-testable. Real-rect size classes are the Android/iOS lesson: a 6x3 on a laptop is smaller than a 4x2 on the 4K workstation.
- **Theme engine**: TOML theme files with semantic roles, `$palette` indirection, named gradients (Oklab-interpolated 64-entry LUTs), glyph tiers (ascii/unicode; nerd never default), border sets and a title style; capability ladder truecolor→256→16→mono resolved once at load; hand-rolled nearest-256; WCAG warnings at load. — Components must never name a colour or glyph or the modern/retrowave split leaks into every renderer. crossterm drops all colour under NO_COLOR, so a mono theme with modifiers+shade ramps is required. `ansi_colours` is LGPL, so the 25-line mapper is written in-tree; `palette` is kept only for Oklab and contrast.
- **Effects**: tachyonfx is deferred to arc 4 behind an `effects` feature; arc 1 ships gradient titles, gradient bars and a paint-the-bg flourish that cost nothing per frame. — Effects are pure polish, are !Send (must live on the render thread anyway), and full-screen ambient effects can blow the VTE write budget; the showcase look in arc 1 comes from colour and glyph choices, which are free.
- **Config and hot reload**: `toml` + `serde` with hand layering (defaults ← file ← env ← CLI), spans for errors; hot reload via a 1 Hz mtime stat thread; edit-mode save (arc 4) via `toml_edit` atomic writes. No figment, no notify. — figment pins toml 0.8 next to toml 1.1; notify 9 is a release candidate and needs directory-watch/debounce ceremony to survive editor renames; a stat per second is free and simpler. `toml_edit` is the only way to save layouts without destroying the user's comments.
- **CPU/htop data**: procfs 0.18 (`default-features = false`) with htop's formulas reimplemented; sysinfo is not used anywhere. — htop parity needs per-class core breakdown, htop's memory formula, PSI, priority/nice/state/NLWP and gated expensive files; sysinfo exposes one number per core and would raise the MSRV to 1.95. Full-scan cost measured on torch is tens of ms at 1.5 s, so no rayon.
- **GPU data**: nvml-wrapper 0.12.1 on its own thread with fast (250 ms) and slow (1 s) tiers; PCIe rates from byte-counter fields, never `pcie_throughput`; a 10-row hand-verified const spec table keyed by PCI id instead of gpuwatch's SQLite; nvidia-smi only when dlopen fails. — Every fast-tier call is sub-microsecond while `pcie_throughput` blocks 21 ms per direction; gpuwatch's DB mislabels the 5090's PCI id; nvidia-smi is an NVML client with the same failure modes at 10 ms per fork.
- **Pins / astral-watch integration**: Pinned git dependency (`rev` dce7eee, `default-features = false`, `[patch]` to the sibling checkout), direct i2c on a dedicated 500 ms thread running astral-watch's own `Lifecycle` with the thresholds from its config files; exporter/CSV modes and upstream PRs (features, log facade, v0.8.0 tag) later; stderr redirected at startup. — astral-watch is not on crates.io and HEAD's public API differs from v0.7.0; the kernel's per-adapter lock makes concurrent reads with the root logger safe; running the same Lifecycle keeps on-screen alarms identical to the service's; library `eprintln!`s would otherwise scribble on the alternate screen.
- **Audio capture**: Spawn `pw-record --format f32 --raw --target auto -P '{ stream.capture.sink = true, node.passive = true }' -` under a supervisor; realfft in-process; cpal/pipewire/libpulse crates banned by cargo-deny. — Verified end-to-end on torch today with ~11 ms latency and zero headers; cpal panics at build time without libasound2-dev; passive streams do not wake idle DACs; a subprocess is also trivially replaceable by the `pulseaudio` crate later.
- **MPRIS and album art**: zbus 5 blocking API on the source thread with two hand-rolled proxies (Position uncached); art decoded with `image` and painted by an in-tree halfblock widget; no ratatui-image, no Sixel/Kitty paths. — `mpris` links libdbus (absent); Ptyxis/VTE 0.84 has no Sixel or Kitty graphics so halfblocks are the only path here and a ▀ painter is 30 lines; ratatui-image's default feature needs libchafa which is not installed.
- **Testing**: insta snapshot matrix (component × footprint × theme) on TestBackend, a no-panic sweep over tiny Rects, JSONL replay fixtures recorded with `--record`, and a `--demo` mode that swaps every source for synthetic data. — Because live, replay and demo all flow through the same `Feed<T>`, renderers can be developed and regression-tested without the hardware; snapshots are the cheapest way to review 5 size classes × 7 components × 3 themes.
- **MSRV and dependency policy**: rust-version 1.88, edition 2024, exact-pinned ecosystem crates per arc, cargo-deny licence and ban lists in CI. — ratatui 0.30.2 declares 1.88; sysinfo's 1.95 is avoided; the MSRV job is an astral-watch convention; the ban list turns the research's 'do not use' findings (tokio, cpal, mpris, LGPL) into a CI failure instead of a code review comment.

## Proposed first arc

Arc 1 = opstui v0.1.0, one session, ending with a screenshot-worthy Overview page in the retrowave theme on the 250×70 Ptyxis terminal. Deliverables: (1) repo scaffold at github.com/mbeaman/opstui — Cargo.toml (lib+bin, rust-version 1.88, features gpu/pins), deny.toml, MIT licence, CHANGELOG, README with runtime requirements, ci.yml (fmt, clippy -D warnings, test, doc, MSRV 1.88, cargo-deny, demo smoke run). (2) Core: `App` render loop with fixed 30 fps cadence and dirty gating, input thread, stderr redirect, chained panic hook, mouse capture opt-out, `--stats` overlay reporting frame time/changed cells/bytes so VTE throughput is measured on the real layout. (3) `grid.rs`: tracks/solve/hit/neighbour with unit tests and the ratatui Overlap cross-check; 24×6 default; borders each/shared/none; dense fallback and the too-small notice. (4) `theme/`: Role/Gradient/Glyphs/Borders, TOML loader with $palette and inherits, truecolor/256/16/mono ladder, WCAG warnings; built-in retrowave, modern and mono; `t` cycles themes. (5) `source/`: Source trait, Feed, spawn loop with backoff and visibility, JSONL Recorder, ReplaySource, DemoSource, `--record`, `--replay`, `--demo`. (6) Components: `clock` (template), `cpu` (procfs sampler with htop formulas; Tiny/Small/Medium/Large classes incl. per-CCD core bars with SMT pairs, Tccd temps, mem/swap stacked bars, load/tasks, read-only top-N process table), `gpu` (NVML worker with fast/slow tiers, static spec table, nvidia-smi fallback; Tiny/Small/Medium/Large incl. nvtop header parity and 50 Hz power sparkline; charts and process table deferred), `pins` (astral-watch git dep, Lifecycle-driven alarms, Tiny/Small/Medium/Large with bars, peak caps, limit line, balance gauge, watts sparkline, log; alert banner/toast overlay). (7) Config: `assets/config.default.toml`, `opstui config check|default`, validation with spans; hot reload deferred. (8) Widgets: stacked_bar, vbars, chip, big, table. (9) Tests: unit tests for grid/theme/htop formulas/spec table; insta matrix for 4 components × footprints × 3 themes; the 0×0..12×5 no-panic sweep; replay tests on fixtures recorded on torch (cpu under the game, gpu under the game, pins idle plus a hand-edited overload fixture). Exit criteria: clippy/fmt/test/doc/MSRV/deny green, `opstui --demo` renders identically in CI, `--stats` p95 frame time under 8 ms on the workstation, and the user approves the v0.1.0 commit after the adversarial review/fix pass.
