> **Status: planning baseline, 2026-08-30.** Written by the design workflow (3 proposals → 3 judges → synthesis → 2 adversarial critics → revision 2) reviewed by Claude, and walked through with Matt on 2026-08-31 — D35 resolves every open decision, so the baseline stands approved for build. Provenance in `docs/design-review/`, evidence in `docs/research/`. Change decisions through `docs/DECISIONS.md`; keep this file current per arc.

# Workspace layout

```
gridwatch/
├── Cargo.toml                      # [workspace]; [workspace.dependencies] single pin per crate; [workspace.package] rust-version = "1.88", edition = "2024", license = "MIT"
├── rust-toolchain.toml             # channel = "stable"
├── clippy.toml                     # disallowed-methods: ratatui::init, ratatui::try_init, ratatui::run, ratatui::restore (the app owns terminal setup/restore)
├── deny.toml                       # cargo-deny: licences MIT/Apache-2.0/BSD/ISC/CC0/Zlib/Unicode/CDLA-Permissive-2.0; bans cpal, mpris, pipewire, libpulse*, ansi_colours, enum-map, sysinfo; duplicate advisories
├── .cargo/config.toml.example      # template for the git-ignored [patch."https://github.com/mbeaman/astral-watch"] → ../astral-watch
├── Makefile                        # ci, demo, shot, insta-review, keys-doc, perf shortcuts
├── scripts/perf/measure.sh         # the PERFORMANCE.md protocol: per-thread pidstat, task-summed voluntary switches, Δwchar from /proc/<pid>/io, nvidia-smi pmon, pw-top; appends a dated row
├── README.md · CHANGELOG.md · CONTRIBUTING.md · LICENSE · THIRD_PARTY.md
├── docs/
│   ├── PLAN.md · DECISIONS.md · BACKLOG.md · MODELS.md · REVIEW.md · MACHINE.md · briefs/ · research/ · design-review/   # the planning corpus, committed from day one
│   ├── ARCHITECTURE.md             # §1–§11 of the architecture, kept current per arc
│   ├── ADDING-A-COMPONENT.md       # the checklist in §15
│   ├── PARITY.md                   # per-tool (htop, nvtop, astral-watch) feature list: in-scope (arc N) / out (reason)
│   ├── PLUGINS.md                  # the public plugin API: manifest, wire protocol, view tree, versioning (arc 8)
│   ├── COMPONENTS.md               # generated from `gridwatch component info`
│   ├── KEYS.md                     # generated from `gridwatch keys` (metric catalogue)
│   ├── THEMES.md · LAYOUT.md · KEYBINDINGS.md · PERFORMANCE.md   # PERFORMANCE holds the measured Ptyxis numbers per page
│   └── img/                        # arc 1: ANSI dump + hand-captured PNG; from arc 2: CI-regenerated SVG screenshots
├── schema/                         # JSON Schemas for every seam: config, layout, theme, journal, view, manifest, exec (validated against fixtures in CI)
├── plugins/examples/                # arc 8: a Python plugin speaking the exec protocol (source + component), the reference for third parties
├── fixtures/
│   ├── journals/                   # torch-idle.jsonl, torch-game.jsonl, torch-audio.jsonl, synth-overload.jsonl (tables off, ≤1 MB each)
│   ├── layouts/                    # default.toml, showcase.toml, laptop-120x40.toml
│   ├── procfs/                     # recorded /proc text for formula tests
│   └── themes/                     # base16 + alacritty samples for the importer tests (arc 9)
├── themes/                         # embedded via include_str!: base-dark.toml, base-light.toml, modern.toml, retrowave.toml, mono.toml, terminal.toml, phosphor-green.toml, phosphor-amber.toml
├── packaging/
│   ├── nfpm.yaml                   # deb/rpm (Recommends pipewire-bin, libnvidia-compute)
│   ├── aur/PKGBUILD
│   ├── udev/90-gridwatch-rapl.rules   # optional chmod 0440 on intel-rapl:0/energy_uj (documented, never required)
│   └── flake.nix                   # arc 9
├── .github/workflows/
│   ├── ci.yml                      # fmt, clippy -D warnings, test, doc -D warnings, MSRV 1.88, per-crate check, feature matrix, deny, tree -d, audit, (arc 2+) demo screenshot
│   └── release.yml                 # gnu + musl tarballs, nfpm, GitHub release from CHANGELOG
└── crates/
    ├── store/        gridwatch-store          # no TUI crate, no crossterm, no system deps; builds in ~1 s
    │   └── src/
    │       ├── lib.rs                      # re-exports
    │       ├── ts.rs · clock.rs            # Ts, Clock (Real | Virtual)
    │       ├── key.rs                      # MetricId, Label, Key<T>, Datum, RecordValue trait + blanket impl, KeyMeta, CATALOGUE, lookup()
    │       ├── ring.rs · series.rs         # Ring<T> (VecDeque), Scalar/Vector/RecordSeries, Retention, resample
    │       ├── store.rs                    # Store::apply + read API, generations, labels (BTreeMap)
    │       ├── msg.rs                      # Msg, Batch, Sample, ControlMsg, Reload, Channels (data bounded / control unbounded / input unbounded)
    │       ├── input.rs                    # InputEvent, KeyEvent, KeyCode, Mods, MouseEvent (serde mirror; converted from crossterm in the app)
    │       ├── source.rs                   # SourceId, Source, AsyncSource, Sampler, SourceCtx, SourceDef, Demand, Level, Cadence, Control, SourceStatus
    │       ├── alert/{mod,event,log,rule,engine}.rs   # AlertEvent, AlertLog, Rule, RuleEngine (name-indexed)
    │       ├── journal/{mod,format,record,replay}.rs  # JSONL header/lines, RecordValue to_json/decode, Recorder tee, Replay + JournalSource
    │       ├── capability.rs               # Capability, CapSet (probe lives in app)
    │       ├── demo/{mod,synth}.rs         # seeded xorshift Synth per source (used by --demo AND by the ui testkit)
    │       └── keys/{mod,sys,cpu,gpu,pins,net,audio,media,sensor}.rs   # typed catalogue, SOURCE consts, Record types (ProcTable, GpuProcs, GpuInfo, NowPlaying, …) with decode entries
    ├── ui/           gridwatch-ui             # ratatui-core + ratatui-widgets only (no crossterm)
    │   └── src/
    │       ├── lib.rs
    │       ├── component.rs                # Component (view → View), Manifest (sources + optional_sources), ComponentDef, Registry, BuildCx, RenderCx/TickCx/InputCx, Outcome, Command, Action
│       ├── view.rs                     # View tree (Text, KeyValue, Gauge, Bars, Sparkline, Chart, Table, BigNumber, Stack, Custom), Span, Paint; serde for the wire protocol
│       ├── renderer/{mod,text,gauge,bars,spark,chart,table,big,stack}.rs   # the default Renderer parameterised by Theme::widgets
    │       ├── tier.rs                     # Footprint, Tier (cumulative, adds), Chrome, view resolution
    │       ├── theme/{mod,file,color,gradient,glyphs,borders,title,effects}.rs   # Theme, ThemeFile, nearest_256/16, WCAG gate (arc 3), EffectHooks (data only, parsed arc 4)
    │       ├── layout/{mod,grid,page,mode,edit,focus}.rs   # tracks/thresholds/solve/hit/unit_at, SolveMode + hysteresis, pure edit ops, spatial focus
    │       ├── widgets/{stacked_bar,vbars,big_number,halfblock,chip,kv_table,proc_table,sparkline_ext,scope,grid_floor,toast,banner}.rs
    │       ├── overlay.rs                  # banner, toasts, help, edit ghosts, stats HUD, too-small notice
    │       ├── dump.rs                     # cells (RLE styled) / ansi / svg (arc 2) buffer dumpers
    │       └── testkit.rs                  # feature "testkit": snapshot_matrix!, role_swatch!, assert_never_panics, assert_min_tier_fits, demo_store() via store::demo, real_grid_sizes()
    ├── sources/      gridwatch-sources        # store + system crates; every module behind a feature
    │   └── src/
    │       ├── lib.rs · registry.rs        # SourceDef table by feature
    │       ├── supervisor.rs · backoff.rs  # spawn_source (catch_unwind, restart counter), spawn_async_runtime (current_thread tokio)
    │       ├── cpu/{mod,stat,mem,psi,procs,topology,freq,k10temp}.rs   # k10temp Tccd by label until sensors exists
    │       ├── gpu/{mod,nvml,fields,specs,smi_fallback,procs}.rs
    │       ├── pins/{mod,i2c,exporter,csv,lifecycle_bridge}.rs
    │       ├── net/{mod,dev,link,addrs,route,dns,conns,probe,wifi}.rs
    │       ├── audio/{mod,pwrecord,supervisor,dsp,bands,scope,vu,pwdump}.rs   # dsp: dual FFT 8192/2048
    │       ├── mpris/{mod,proxy,discovery,player,art}.rs
    │       └── sensors/{mod,hwmon,rapl,cpufreq}.rs
    ├── components/   gridwatch-components     # store + ui; every module behind a feature
    │   └── src/
    │       ├── lib.rs · registry.rs        # ComponentDef table by feature
    │       ├── clock.rs                    # 60-line template (Chrome::Borderless)
    │       ├── sources.rs · alerts.rs      # debugging tiles
    │       ├── htop/{mod,manifest,options,state,tiers,keys,actions}.rs
    │       ├── gpu/{mod,manifest,tiers,charts,procs}.rs
    │       ├── pins/{mod,manifest,tiers,log}.rs
    │       ├── net/{mod,manifest,options,tiers,conns}.rs
    │       ├── audio/{mod,manifest,ballistics,tiers}.rs
    │       ├── winamp/{mod,manifest,skin,marquee,tiers,playlist}.rs
    │       └── sensors/{mod,manifest,tiers}.rs
    ├── app/          gridwatch-app            # ratatui facade + crossterm 0.29 + tachyonfx (arc 4)
    │   └── src/
    │       ├── lib.rs                      # pub fn run<B: Backend>(terminal: &mut Terminal<B>, Registry, Cli) -> Result<()>
    │       ├── terminal.rs                 # enable_raw_mode + EnterAlternateScreen/EnableMouseCapture/EnableFocusChange, restore, PanicPolicy hook, stderr dup2
    │       ├── loop.rs                     # drain input → control → data(≤3 ms) → solve → demand → tick → dirty → draw
    │       ├── app.rs · pages.rs · focus.rs · edit.rs · zoom.rs
    │       ├── probe.rs                    # CapSet probe (≤200 ms), doctor table
    │       ├── config/{mod,behaviour,layout,rules,layering,validate,watch,save}.rs   # toml + serde, spans, options disjointness, 1 Hz mtime watcher, toml_edit save
    │       ├── effects.rs                  # EffectHooks → tachyonfx, budget watchdog (arc 4)
    │       ├── executor.rs                 # Action runner thread → ControlMsg::Done
    │       ├── input.rs                    # the one event::read() thread; crossterm::Event → store::InputEvent
    │       └── stats.rs                    # frame time p50/p95, changed cells, bytes, mode
    └── cli/          gridwatch                # the binary
        └── src/
            ├── main.rs                     # builtin_registry() by features → terminal setup → gridwatch_app::run
            ├── cli.rs                      # clap: run [--demo [--seed]|--replay F --speed N|--record F|--page|--theme|--fps|--color|--no-mouse|--stats|--stats-log F], shot, config check|default|explain, component list|info, keys, doctor, theme import
            └── registry.rs
```

## Cargo.toml (workspace dependencies)

```toml
[workspace]
members = ["crates/store", "crates/ui", "crates/sources", "crates/components", "crates/app", "crates/cli"]
resolver = "3"

[workspace.package]
edition = "2024"
rust-version = "1.88"
license = "MIT"
repository = "https://github.com/mbeaman/gridwatch"

[workspace.dependencies]
# TUI
ratatui          = { version = "0.30.2", features = ["serde", "palette"] }        # app + cli only (defaults: crossterm 0.29 backend, macros, layout-cache, underline-color)
ratatui-core     = { version = "0.1.2", features = ["std", "serde", "underline-color", "layout-cache"] }   # ui, components (default = [] upstream)
ratatui-widgets  = { version = "0.3.2", default-features = false, features = ["std", "serde"] }            # ui, components (drops the calendar/time dependency)
crossterm        = "0.29.0"                                                       # app + cli only; matches ratatui-crossterm's crossterm_0_29 so `cargo tree -d` stays clean
tachyonfx        = "0.25.1"                                                       # app only (effects, !Send), arc 4
tui-big-text     = "0.8.9"                                                        # ui widgets (clock, Winamp digits)
unicode-width    = "0.2.2"
# data / config
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"                                                            # journal lines, RecordValue::to_json
toml             = "1.1.4"                                                        # config/theme parsing with spans
toml_edit        = "0.25.13"                                                      # comment-preserving layout save (arc 4)
palette          = { version = "0.7.7", features = ["std"] }                     # Oklab gradients, WCAG contrast
smallvec         = "1.15"
color-eyre       = "0.6.5"
clap             = { version = "4.6.6", features = ["derive"] }
tracing          = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
libc             = "0.2"                                                          # dup2, setpriority, ioprio_set (never the 1.0 alpha)
# sources (all optional at the crate level)
procfs           = { version = "0.18.0", default-features = false }               # feature cpu, net
nvml-wrapper     = "0.12.1"                                                       # feature gpu (dlopens libnvidia-ml.so.1)
astral-watch     = { git = "https://github.com/mbeaman/astral-watch", rev = "dce7eee77676268c66b3624c7a2870ed9d84eb9c", default-features = false }  # feature pins; tag v0.8.0 once cut
realfft          = "3.5.0"                                                        # feature audio (dual FFT 8192/2048)
rtrb             = "0.4.0"                                                        # feature audio (SPSC ring)
ebur128          = "0.1.10"                                                       # feature audio-lufs
zbus             = { version = "5.19.0", default-features = false, features = ["tokio"] }   # features mpris, net-dns
tokio            = { version = "1.53", features = ["rt", "sync", "time", "macros"] }        # only inside gridwatch-sources behind mpris / net-probe / net-rdns / net-dns
surge-ping       = "0.9.0"                                                        # feature net-probe
hickory-resolver = "0.26.1"                                                       # feature net-rdns
neli-wifi        = "0.6.1"                                                        # feature net-wifi
nix              = { version = "0.31.3", features = ["signal", "sched", "net"] }  # process actions, ifaddrs
uzers            = "0.12.2"                                                       # uid → name
image            = { version = "0.25", default-features = false, features = ["png", "jpeg", "webp"] }   # feature mpris (album art)
ureq             = { version = "3.4", default-features = false, features = ["rustls", "gzip"] }          # feature mpris (https art), net public IP — brings ring (ISC) + webpki-roots (CDLA-Permissive-2.0)
caps             = "0.5.6"                                                        # capability probe
# dev
insta            = "1.48"
proptest         = "1.11"
criterion        = "0.8"
```

Per-crate features: `gridwatch-sources` and `gridwatch-components` expose `cpu, gpu, pins, net, net-probe, net-rdns, net-dns, net-wifi, audio, audio-lufs, mpris, sensors`; `gridwatch` (bin) `default = ["cpu", "gpu", "pins", "net", "net-probe", "audio", "mpris", "sensors"]` (demo is always built — it lives in the store); `gridwatch-ui` exposes `testkit = ["dep:insta", "dep:proptest"]`. Deliberately absent: sysinfo (MSRV 1.95, no class breakdown), cpal/pipewire/libpulse (headers missing), `mpris` crate (libdbus), ratatui-image (halfblocks are 30 lines; default feature needs libchafa), figment (toml 0.8 duplicate), notify (1 Hz stat instead), enum-map (3.x needs 1.95), ansi_colours (LGPL), prometheus-parse (50-line parser in-tree). crossterm never appears below `gridwatch-app`.
