<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Prior-art survey for opsTui: what existing system/GPU/network/audio/player TUIs do well, what to avoid, and which parts are consumable as Rust crates

## Headline findings (things that change the plan)

1. **amdgpu_top is NOT ratatui-based.** Its TUI crate `amdgpu_top_tui` depends on `cursive = 0.21` (crossterm backend) + `termsize` (verified from `crates/amdgpu_top_tui/Cargo.toml`). Its UI *structure* is still worth copying (see below), but no code is reusable for a ratatui app.
2. **bottom's `[lib]` target is internal.** `Cargo.toml` defines `[lib]` + `[[bin]] btm`, but docs.rs says the docs are "for development purposes"; the only entry point is `start_bottom()`. Treat bottom as a reference, not a dependency. Same for `trippy-tui` (app-specific) and rmpc (workspace, `rmpc-mpd` on crates.io is a `0.0.0` placeholder).
3. **On this machine, album art / pixel graphics is halfblocks-only.** `strings libvte-2.91-gtk4.so.0` gives the feature string `+BIDI +GNUTLS +ICU +SYSTEMD` — no `+SIXEL`, and VTE has no Kitty-graphics implementation. None of kitty/ghostty/wezterm/foot/alacritty/ueberzugpp/chafa are installed. rmpc-style Kitty/Sixel art will silently fail in Ptyxis; `ratatui-image` must be used with `Picker::from_fontsize(..)` → `Halfblocks`, not `from_query_stdio()`.
4. **cpal cannot build here.** cpal 0.18.2 requires `libasound2-dev` on Linux *even when* its optional `pipewire`/`pulseaudio` features are used (README, verified); `pkg-config --exists alsa|libpipewire-0.3|libpulse` all fail locally. The zero-header capture path that works today: spawn `pw-record --target <monitor> --format s16 --rate 48000 --channels 2 --raw -` (flags verified from `pw-record --help`) or `ffmpeg -f pulse -i default …` (ffmpeg lists `alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink.monitor` as the default `*` source), and do the FFT in-process with `realfft`/`rustfft`. rmpc does exactly this kind of subprocess integration (it spawns `cava` in raw mode over a FIFO).
5. **`mpris` crate (used by fum) pulls in `dbus`/libdbus** (feature `dbus-vendored = [dbus/vendored]`); use `zbus 5` as ncspot does (`mpris = ["zbus"]` feature).

## Survey table (all metadata from GitHub API on 2026-08-30 unless noted)

| Project | Lang / UI lib | License | Latest release / last push | Distinctive; what to steal |
|---|---|---|---|---|
| **btop** (aristocratos/btop) | C++23, custom renderer (no ncurses) | Apache-2.0 | v1.4.7 2026-05-01 / 2026-08-30, 34.3k★ | `presets="cpu:1:default,proc:0:default cpu:0:default,mem:0:default,net:0:default"` (box:Position:GraphSymbol, up to 9, hotkeys), `shown_boxes` incl. `gpu0..gpu5`, `graph_symbol` braille/block/tty, `truecolor`/`force_tty` 16-colour downgrade, `terminal_sync`, `rounded_corners`, full mouse, `vim_keys`, `net_auto`/`net_sync` scaling, `cpu_graph_upper/lower`. Theme files: 41 keys (`main_bg, main_fg, title, hi_fg, selected_bg/fg, inactive_fg, graph_text, meter_bg, proc_misc, cpu_box, mem_box, net_box, proc_box, div_line` + `*_start/_mid/_end` gradients for temp, cpu, free, cached, available, used, download, upload, process). GPU via dlopen of NVML/ROCm. |
| **bottom** (ClementTsang/bottom) | Rust, ratatui 0.30.2 + crossterm 0.29.0 + ratatui-core 0.1.2, sysinfo =0.39.6, nvml-wrapper 0.12.1 (opt), MSRV 1.95.0 | MIT | 0.14.9 2026-08-27 / 2026-08-30, 13.9k★ | Layout TOML: `[[row]] ratio=2 / [[row.child]] ratio=4 type="mem" / [[row.child.child]] type="temp"`; types cpu, mem, net, proc, temp, temp_graph, disk, empty, batt. Internals (`src/app/layout_manager.rs`): `BottomLayout > BottomRow > BottomCol > BottomColRow > BottomWidget{left/right/up/down_neighbour: Option<u64>}` with neighbours computed geometrically (`get_movement_mappings`, `is_intersecting`, `get_distance`). `[styles]` with 6 built-in themes and sub-tables `styles.cpu/memory/network/battery/tables/graphs/widgets`; colours as named/`#hex`/`"r, g, b"`, `{ colour, bg_colour, bold }`. `src/canvas/components/{data_table, pipe_gauge, time_series (vendored ratatui canvas), widget_carousel, scroll_bar}` and `widgets/{cpu_basic, cpu_graph, mem_basic, network_basic, …}` = explicit "basic" footprints per widget. Widget carousel: `◄ name` / `name ►` cycling in one slot with mouse bounds. |
| **zenith** | Rust, ratatui 0.29/crossterm 0.28/sysinfo 0.37/nvml-wrapper 0.10 (opt) | MIT | 0.15.0 2026-05-08 / 2026-08-25 | Zoomable charts with scroll-back in time; history persisted between runs; per-process disk I/O; per-process GPU. |
| **gotop** (xxxserxxx) | Go, gizak/termui | (NOASSERTION) | v4.2.0 2022-09-29 / 2026-05-07 | Layout DSL `(rowspan:)?widget(/weight)?` per line, e.g. default `2:cpu` / `disk/1  2:mem/2` / `temp` / `2:net  2:procs`; kitchensink adds `power`. Go plugins abandoned as "hacky"; extensions moved in-tree. |
| ytop | Rust | MIT | archived 2020-08-29 → "use bottom" | — |
| **glances** | Python, curses | LGPL-3.0 | v4.5.6 2026-08-01, 33.5k★ | Plugin + export architecture (CSV/JSON/InfluxDB/Prometheus/Kafka…), REST/Web/MCP server, careful/warning/critical thresholds in `glances.conf`. Anti-pattern: Python start-up latency. |
| bpytop / bashtop | Python / Bash | Apache-2.0 | superseded by btop (last push 2025-06 / 2023-08) | Same theme format as btop. |
| nmon | C (single 8,600-line file), curses | GPL | site says 16s (2015) | Single-key section toggles (c/m/n/d/t) stacking panels; capture-to-CSV mode. |
| **s-tui** | Python, urwid | GPL-2.0 | v1.5.0 2026-08-19 | CPU freq/util/temp/power graphs + stress-test integration + per-core throttle reasons. |
| **htop** (local 3.4.1; latest 3.5.3 2026-08-16) | C, ncurses | GPL-2.0 | / 2026-08-30 | Verified in `~/.config/htop/htoprc`: `header_layout=two_50_50`, `column_meters_0=LeftCPUs4 Memory Swap`, `column_meter_modes_0=1 1 1`, `screen_tabs=1`, `screen:Main=PID USER …` + `.sort_key`. `MeterMode.h`: `BAR_METERMODE=1, TEXT, GRAPH, LED` — the canonical "same meter, four footprints" pattern. File header literally says "This file is rewritten by htop… The parser is also very primitive" (anti-pattern). |
| **nvtop** (local 3.2.0; latest 3.3.2 2026-02-08) | C, ncursesw | GPL-3.0+ | / 2026-05-06 | F2 setup (General: colours/interval; Devices: temp scale, ENC/DEC hide timer; Chart: reverse plot, metric selection; Processes: sort/columns) → F12 persists to `~/.config/nvtop/interface.ini` ("Do not edit"). Dynamic ENC/DEC meters appear only while active. `-p` single max-of-all-GPUs bar, `-d` tenths-of-seconds delay. |
| **nvitop** | Python, curses | Apache-2.0 (API) / GPL-3.0 (CLI) | v1.7.1 2026-07-10 | `full`/`compact` auto by terminal size; `--colorful` spectrum bars; process tree; library API `nvitop.Device`, `ResourceMetricCollector`, `take_snapshots()`. |
| gpustat | Python (nvidia-ml-py) | MIT | v1.1.1 2023-08-22 | One line per GPU `[0] name | 77°C, 96 % | 11848 / 12287 MB`, `--json`, `--watch` — the 1x1 footprint. |
| **amdgpu_top** (local 0.11.2; latest v0.11.5 2026-05-18) | Rust, **cursive 0.21** (crossterm) | MIT | / 2026-08-22 | Workspace: `libamdgpu_top` (lib), `amdgpu_top_tui`, `_gui` (egui), `_json`. TUI: `TuiApp`/`SuspendedTuiApp` (device gone → keep running, reattach), `AppLayout`, `LinearLayout::vertical/horizontal` switched at `WIDE_TERM_COLS = 150`, `Panel`, custom `PerfCounterView/VramUsageView/ActivityView/AppTextView`, view modules `activity, fdinfo, gpu_metrics, sensors, vram, perf_counter, memory_error_count, xdna_fdinfo`; toggle keys `g r v a f`, sort `P V G M`, `T` theme; `smi.rs` = compact nvidia-smi-like `SmiApp` mode with `p` to toggle processes. |
| qmassa | Rust (ratatui per awesome list) | Apache-2.0 | 332 commits | GPU TUI + Prometheus exporter in one workspace (`qmassa`, `qmlib`, `qmmd`). |
| **wtfutil** | Go, tcell/tview | MPL-2.0 | v0.50.0 2026-06-30, 17k★ (renaming to Tessera) | YAML: `wtf.grid.columns: [35,35,35,35]`, `rows: [10,10,10,10,4]` (absolute chars), module `position: {top, left, height, width}`, per-module `refreshInterval`, `colors.border.{focusable,focused,normal}`. |
| **sampler** | Go, gizak/termui v3 | GPL-3.0 | v1.1.0 2019-12-24 / 2024-02 (dormant), 14.8k★ | YAML components `runchart, sparkline, barchart, gauge, textbox, asciibox`; `position: [[x,y],[w,h]]`; `rate-ms`, `sample:` shell cmd, `init`, `transform`, `triggers`, `pty`, `theme: light/dark`. |
| **termdash** | Go | Apache-2.0 | v0.20.0 2024-03-10 / 2026-08-24 | Container binary tree (`SplitVertical/Horizontal`) + grid builder (`RowHeightPerc/ColWidthPerc`); widgets dir: barchart, borderfx, button, checkbox, donut, dropdown, fx, gauge, heatmap, linechart, modal, pie, radar, radio, segmentdisplay, slider, sparkline, spectrum, spinner, tab, text, textinput, threed, timeline, toast, treeview. |
| jpterm / tiptop / Textual | Python, Textual (v8.2.8 2026-06-30) | MIT | jpterm 0.3.10 2025-12; tiptop v0.2.8 2022-09 (dormant) | Textual CSS-driven layouts; jpterm `txl` plugin system. |
| **bandwhich** | Rust (ratatui) | MIT | v0.23.1 2024-10-08, "passive maintenance" | Per-process/connection/remote bandwidth; needs `cap_sys_ptrace,cap_dac_read_search,cap_net_raw,cap_net_admin` or sudo; raw machine-readable mode; unit families. |
| sniffnet | Rust, **iced GUI** (not TUI) | Apache-2.0/MIT | v1.5.1 2026-07-22, 41k★ | Custom themes, thumbnail mode — reference only. |
| **trippy** | Rust, ratatui; workspace `trippy-core/-tui/-dns/-packet/-privilege` | Apache-2.0 | 0.13.0 2025-05-05 / 2026-08-27 | TOML `[theme-colors]` ~40 keys (`bg-color, border-color, hops-chart-selected-color, samples-chart-color, map-world-color, info-bar-bg-color…`), `[bindings]` (`toggle-help="h"`), `tui-refresh-rate="100ms"`; CAP_NET_RAW. |
| **gping** | Rust (ratatui) | MIT | gping-v1.20.4 2026-06-24 | Braille latency chart, `--simple-graphics` dot fallback, `--cmd` graphs any command's latency, hex/named colours. |
| bmon | C, curses | BSD/MIT | v4.0 2016-12-13 / 2026-08 | netlink stats, output modules (curses/ascii/format). |
| **cava** | C | MIT | 1.0.0 2026-06-13 | PipeWire default input; outputs ncurses/noncurses/raw/sdl/sdl_glsl; `raw` mode writes bar values to stdout for other programs; `autosens`, `sensitivity`, `monstercat`, `waves`, `noise_reduction`, `lower/higher_cutoff_freq`, gradients. |
| cli-visualizer ("vis") | C++14, ncursesw + fftw | MIT | author deleted GitHub account; mirrors PosixAlchemist/cli-visualizer, 69keks/cli-visualizer-git | Modes spectrum/ellipse/lorenz; MPD FIFO 44100:16:2; `monstercat`/`sgs` smoothing; exponential falloff; colour files in `~/.vis/colors/`. |
| glava | C + GLSL, OpenGL 4.3, X11 only | GPLv3+MIT | v1.6.3 2019-03-12 / 2024-01 (dormant) | Modules radial/bars/graph/circle/wave — visual reference only. |
| **scope-tui** 0.3.5 | Rust; ratatui 0.29 (opt), rustfft 6.3, cpal 0.15 (opt), libpulse-simple-binding (opt) | MIT | crates.io | Oscilloscope (trigger/edge), spectroscope (log axis, Hann), vectorscope. |
| rav (i-am-logger/rav) | Rust, ratatui | **CC BY-NC-SA 4.0** (do not copy code) | pushed 2026-08-24 | Themes `rav/winamp/terminal/mono`; ballistics + ramp ported from Webamp (MIT); ALSA capture (needs alsa-lib). |
| audioviz 0.6.0 (codeberg, BrunoWallner) | Rust | MIT | 2026-08-10 | `spectrum`, `distributor`, `fft`, cpal capture behind feature; only 33% documented. |
| **ncspot** | Rust, cursive | BSD-2-Clause | v1.4.0 2026-08-21 | MPRIS via `zbus` feature (no libdbus), cover via `viuer`, IPC socket, vim keys. |
| spotify-tui | Rust, tui-rs | MIT | v0.25.0 2021-08-24 / 2024-04 (dormant) | YAML theme keys active/selected/hovered; keybinding map. |
| **rmpc** | Rust, ratatui 0.30.0 + crossterm 0.29 (osc52) + image 0.25.9 + tokio 1.49; MSRV 1.97.1 on master | BSD-3-Clause | v0.11.0 2026-02-01 / 2026-08-28 | RON theme with layout tree: `layout: Split(direction: Vertical, panes: [(pane: Pane(Header), size: "2"), (pane: Pane(Tabs), size: "3"), (pane: Pane(TabContent), size: "100%"), (pane: Pane(ProgressBar), size: "1")])`; sizes `"N"` cells or `"N%"`; `components: Map<String, Pane|Split|Component>` referenced as `Component("name")`; pane types AlbumArt, Lyrics, ProgressBar, Header, Tabs, Cava, TabContent (exactly one). Own image backends `src/ui/image/{kitty,iterm2,sixel,ueberzug,block,facade}.rs` — `AlbumArtFacade` wrapping `enum ImageBackend`, `show/display/hide/cleanup/set_size`, encoding on a worker thread; `Block` = halfblock fallback. Cava pane spawns the `cava` binary (raw output) fed by an MPD FIFO; theme keys `bar_symbols: ['▁'…'█']`, `inverted_bar_symbols`, `bar_width`, `bar_spacing`, `orientation`, `bar_color` (single/per-row/gradient). Widgets: `virtualized_table, scrolling_line, progress_bar, volume, tabs, button, input`. |
| termusic | Rust, tui-realm (tuirealm 4.1.0 wraps ratatui 0.30) | MIT (+GPL podcast) | v0.13.2 2026-05-06, MSRV 1.90 | Server/client over gRPC; cover via kitty/iterm/sixel/ueberzug; yazi/alacritty theme import. |
| musikcube / cmus | C++ / C, ncurses | BSD-3 / GPL-2 | 3.0.5 2025-09-21 / v2.12.0 2024-10-26 | musikcube daemon + remote clients; cmus `.theme` = `set color_*` lines, format strings `%a %t`. |
| **fum** | Rust, ratatui 0.29, ratatui-image 4.1.0, `mpris 2.0.1` (libdbus) | MIT | 283★, mid-rewrite (#98) | JSONC `~/.config/fum/config.jsonc` widget tree: `Container{direction, flex: start/center/end/space-around/space-between}`, `CoverArt{resize: fit/crop/scale}`, `Label`, `Button` (actions/shell), `Progress{filled/empty chars}`, `Volume`, `Empty`; placeholders `$title $artists $album $status-icon $position $length`; `players: [mpris names]`, `use_active_player`; `keybinds: {"esc;q": "quit()"}`. Closest existing thing to a "Winamp component over MPRIS". |
| cliamp | Go (Bubbletea/Lip Gloss) + bundled C++ | MIT | v1.63.2 2026-08-13, 3.9k★ | "Winamp-inspired" player with spectrum + parametric EQ; UX reference only. |
| Winamp classic skin format (webamp, TS/MIT, as reference) | — | — | — | `VISCOLOR.TXT`: 24 RGB rows — 0 background, 1 grid dots, 2–17 spectrum gradient **peak→base**, 18–22 oscilloscope trough→crest, 23 peak-cap marker. `PLEDIT.TXT`: `[Text] Normal, Current, NormalBG, SelectedBG, MbFG, MbBG, Font`. These map 1:1 onto a `retrowave`/`winamp` theme token set. |
| cool-retro-term | QML/C++ | GPL | 2.0.0-beta2 2026-05-31 | CRT effects: scanlines, bloom, burn-in, jitter, curvature, chromatic aberration, static noise, ambient light, flicker; amber/green/IBM-DOS profiles. In a TUI only colour-level analogues are possible (phosphor palettes, dimmed "burn-in" trails, tachyonfx glitch/hsl-shift). |

## Reusable Rust crates (verified via `cargo info`/docs.rs)

- **ratatui 0.30.2** (modular: `ratatui-core 0.1.2`, `ratatui-widgets 0.3.2`, `ratatui-crossterm 0.1.2`); `ratatui::run/init/restore`, `Rect::layout()`, `centered()`, `Flex::SpaceEvenly`, `Marker::{Braille,Quadrant,Sextant,Octant}`, Block border merging. Widget crates should depend on `ratatui-core`.
- **tui-widgets 0.7.11** (MIT/Apache, MSRV 1.88) feature-gated: `tui-bar-graph 0.3.5` (`BarGraph::new(data).with_gradient(colorgrad::preset::turbo()).with_bar_style(BarStyle::Braille).with_color_mode(ColorMode::VerticalGradient)`), `tui-equalizer 0.2.3` (`Equalizer{bands: Vec<Band /*0.0..=1.0*/>, brightness}`), `tui-big-text 0.8.9` (`BigText::builder().pixel_size(PixelSize::{Full,HalfHeight,HalfWidth,Quadrant,ThirdHeight,Sextant,QuarterHeight,Octant})` from font8x8 — Winamp time digits), `tui-scrollview 0.6.7`, `tui-popup`, `tui-cards`, `tui-box-text`, `tui-qrcode`, `tui-prompts`, `tui-scrollbar`.
- **ratatui-image 11.0.6** (MIT, MSRV 1.86): `Picker::from_query_stdio()` / `Picker::from_fontsize(FontSize)`, `Picker::new_protocol(image, area, Resize::{Fit(Option<FilterType>),Crop,Scale})`, `Image::new(&Protocol)`, `StatefulImage`, `ThreadProtocol` for off-thread encode; `Protocol::{Halfblocks,Sixel,Kitty,iTerm2}`; features `crossterm`, `chafa-dyn/static`.
- **tachyonfx 0.25.1** (MIT): `Effect`, `Shader` trait, `EffectManager`, `EffectTimer`, `fx::*` constructors, `dsl`, `pattern`, `wave`, `CellFilter`, `Interpolation` — retrowave transitions/glitch.
- Also available: `tui-logger 0.18.3`, `malevich 1.20.0` (plots), `tui-piechart 1.0.2`, `tui-slider 0.3.3`, `tui-rain 1.0.1`, `tui-shimmer 0.1.3`, `tui-skeleton 0.3.0`, `ratatui-cheese 0.7.0`, `ratatui-garnish 0.1.0`, `throbber-widgets-tui 0.11.1`, `tuirealm 4.1.0` (framework — not recommended, see anti-patterns), `scope-tui 0.3.5` (app, not a widget lib).
- Audio DSP without system headers: `realfft 3.5.0` / `rustfft 6.4.1` / `spectrum-analyzer 1.8.0`; `audioviz 0.6.0` with `spectrum` feature but *without* `cpal`.
- Not reusable: `bottom` lib (internal), `amdgpu_top_tui` (cursive), `rmpc-mpd` (placeholder), `rav` (NC licence), `mpris 2.1.0` (libdbus), `cpal`/`pipewire`/`libpulse-simple-binding` (missing headers).

## Ranked: 10 patterns opsTui should adopt

1. **Declarative layout tree + named presets + reusable components** — bottom's `[[row]]/[[row.child]]/ratio/type` TOML, rmpc's `Split/Pane/size("40%"|"3")` + `Component("name")`, gotop's rowspan/weight DSL, btop's 9 hot-key `presets`. Ratio-based, never absolute cells.
2. **Footprint-adaptive components** — htop meter modes (Bar/Text/Graph/LED), nvitop full/compact auto, amdgpu_top `WIDE_TERM_COLS` switch + SMI mode, bottom `*_basic` widgets, gpustat one-liner. Each component declares supported footprints (6x3/4x2/1x1) and degrades gracefully.
3. **Semantic theme tokens with gradients + colour-depth downgrade** — btop's 41 keys with `_start/_mid/_end`, bottom `[styles.*]`, trippy `[theme-colors]`, Winamp `VISCOLOR.TXT` 24-row ramp; btop `truecolor`/`force_tty`.
4. **Glyph-set abstraction (braille / block / ASCII)** — btop `graph_symbol`, gping `--simple-graphics`, rmpc `bar_symbols`; mandatory here because no Nerd Fonts are installed.
5. **Data collection off the render thread with immutable snapshots** — bottom collection→canvas split, rmpc worker thread for image encode, `ThreadProtocol`, nvitop `ResourceMetricCollector`, astral-watch `spawn_gpu_poller`.
6. **Geometric focus navigation + mouse hit boxes** — bottom `get_movement_mappings` neighbour ids, btop clickable elements, vim keys as option.
7. **Single-key section toggles and quick presets** — nmon c/m/n/d/t, amdgpu_top g/r/v/a/f, btop `shown_boxes` + number keys.
8. **History with zoom/scroll-back and auto-scaling** — zenith zoomable charts + persisted history, btop `net_auto/net_sync`, nvtop reverse-plot, cava `autosens`, Winamp peak-cap ballistics (from webamp, MIT).
9. **In-app setup persisted to config, config still human-editable** — nvtop F2/F12, htop Setup; but write a clean TOML (unlike htoprc).
10. **Machine-readable side outputs** — cava raw, gpustat `--json`, bandwhich raw, glances exporters, astral-watch Prometheus — `--once/--json` and an exporter make components testable.

## 5 anti-patterns observed

1. **Config sprawl / machine-owned config** — htoprc ("rewritten by htop… parser very primitive"), btop's ~80 flat keys, nvtop's "Do not edit this file".
2. **Absolute-cell grids** — wtfutil `columns: [35,35,35,35]`, sampler `[[x,y],[w,h]]` — break on resize/fonts.
3. **System-header and privilege dependencies** — cpal (libasound2-dev), pipewire/libpulse crates, `mpris`→libdbus, bandwhich's four capabilities, glava X11-only, sniffnet drifting to a GUI.
4. **Terminal-capability assumptions** — Kitty/Sixel album art, Nerd-Font icons, Octant/Sextant glyphs; Ptyxis/VTE here has none of the graphics protocols.
5. **Framework lock-in and bit-rot** — cursive (amdgpu_top), tui-rs (spotify-tui dormant, scope-tui README stale), tui-realm; gotop's abandoned Go plugin system; sampler/gotop dormant; nmon's single 8,600-line file and astral-watch's 1,695-line `tui.rs`.

## Implications for the architecture

```rust
// component contract borrowed from htop meters + bottom widgets + rmpc panes
pub enum Footprint { Tile1x1, Wide4x2, Panel6x3, Custom(u16, u16) }
pub trait Component {
    fn id(&self) -> &'static str;
    fn supported(&self) -> &'static [Footprint];
    fn poll(&mut self, snap: &Snapshot);          // called on the tick thread, no drawing
    fn render(&self, area: Rect, fp: Footprint, theme: &Theme, glyphs: &GlyphSet, buf: &mut Buffer);
    fn handle(&mut self, ev: &Event) -> Option<Action>;
}
```
Layout config: bottom-style TOML rows/children/ratio plus rmpc-style `components` map and btop-style `presets` bound to number keys. Theme: token struct with `Gradient { start, mid, end }` fields mirroring btop, a `glyphs = "braille"|"block"|"ascii"` selector, and a `depth = truecolor|256|16` downgrade. Audio: `pw-record … --raw -` subprocess → `realfft` → cava-style `autosens` + monstercat smoothing → `tui-bar-graph`/custom `bar_symbols`. Media: `zbus 5` MPRIS + `ratatui-image` Halfblocks. GPU: `nvml-wrapper 0.12.1` (as bottom/zenith), pins via `astral_watch` crate API. Effects: `tachyonfx` gated by theme.

## Recommendations

- **Model the layout as a ratio-based tree in TOML (bottom's [[row]]/[[row.child]]/ratio/type) with an rmpc-style `components` map for reuse and btop-style numbered `presets`; never absolute cell grids.** — bottom (13.9k★, ratatui 0.30.2) and rmpc (ratatui 0.30) prove this works on ratatui's Constraint system; wtfutil/sampler's absolute grids break on resize; gotop's DSL is compact but not TOML-friendly.
  - alternatives: gotop text DSL (cute, but hand-parsed); wtfutil YAML absolute grid; rmpc RON (RON is less familiar than TOML for the user's existing tooling).
- **Give every component an explicit footprint enum and render function (htop Bar/Text/Graph/LED meter-mode pattern) rather than a single `render(area)` that guesses.** — htop MeterMode.h, nvitop full/compact, amdgpu_top WIDE_TERM_COLS=150 + SMI mode, bottom's *_basic widgets and gpustat's one-liner all show the same widget needs 3-4 designed footprints; guessing from Rect leads to unreadable tiles.
  - alternatives: Responsive-only rendering from Rect size (bottom mostly does this) — acceptable as the fallback inside each footprint.
- **Theme = semantic token file with start/mid/end gradients (port btop's 41-key vocabulary), a glyph-set selector (braille/block/ascii) and a colour-depth downgrade; ship retrowave/winamp using the VISCOLOR.TXT 24-row ramp.** — btop themes (Apache-2.0) are the richest existing vocabulary and dozens exist to port; gradients drive per-cell colouring for bars/graphs; no Nerd Fonts installed so glyph sets must be selectable; VTE truecolor works but a 16-colour TTY fallback is cheap.
  - alternatives: bottom's flat [styles.*] tables (fewer gradients); trippy's ~40 flat keys; rav's TOML themes (NC licence, don't copy).
- **Capture audio by spawning `pw-record --target <sink-monitor> --format s16 --rate 48000 --channels 2 --raw -` (or `ffmpeg -f pulse -i default`) and do FFT in-process with realfft; make cpal a feature flag, not a default dependency.** — cpal 0.18.2 hard-requires libasound2-dev even for its pipewire/pulseaudio features and those headers are absent; pipewire/libpulse crates need headers too; rmpc already ships a subprocess-based cava pane so the pattern is proven; realfft/rustfft/spectrum-analyzer are pure Rust.
  - alternatives: Spawn `cava -p <cfg>` in raw mode (extra runtime dep, but gives autosens/monstercat for free); `arecord -D default` via pipewire-alsa; require the user to apt install libasound2-dev.
- **MPRIS via zbus 5.19 (as ncspot does), album art via ratatui-image 11 with `Picker::from_fontsize` → Halfblocks by default and protocol detection only behind an opt-in flag.** — The `mpris` crate (used by fum) links libdbus (headers absent); libvte here reports `+BIDI +GNUTLS +ICU +SYSTEMD` (no Sixel) and VTE has no Kitty graphics, so rmpc-style art would silently fail in Ptyxis; `from_query_stdio` can stall on non-answering terminals.
  - alternatives: Ueberzugpp overlay (not installed; Wayland layer hacks); install kitty/ghostty for demos (none installed today).
- **Do not depend on bottom, trippy-tui, amdgpu_top or rmpc as libraries; borrow their patterns and vendor small pieces (with attribution) where licences allow (MIT/Apache/BSD).** — bottom's [lib] is declared internal (docs.rs: 'for development purposes'); amdgpu_top_tui is cursive-based; rmpc-mpd on crates.io is a 0.0.0 placeholder; trippy-tui is app-specific.
  - alternatives: tuirealm 4.1.0 component framework (adds an Elm-style layer and version coupling; termusic's experience shows upgrade friction).
- **Use tui-widgets sub-crates (tui-bar-graph, tui-equalizer, tui-big-text) and tachyonfx for showcase pieces; write component crates against ratatui-core 0.1.2.** — All are MIT/Apache, maintained by the ratatui org (last releases June–Aug 2026), and ratatui 0.30's split lets widget crates avoid the full ratatui dependency.
  - alternatives: Hand-rolled bar/spectrum widgets (rmpc's cava pane is ~1 file and gives full control over bar_symbols/gradients — reasonable for the Winamp component).
- **Keep components in-tree behind Cargo features (like gotop after it abandoned Go plugins) rather than runtime plugins; expose `--once/--json` and a Prometheus exporter for testability.** — gotop documents runtime plugins as 'problematic/hacky'; glances/cava/gpustat/bandwhich all benefit from machine-readable modes; astral-watch already has an exporter pattern to mirror.
  - alternatives: Dynamic plugins (dlopen) or WASM components — unjustified complexity for a personal dashboard.

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `ratatui` | 0.30.2 | Core TUI framework; modular ratatui-core 0.1.2 / ratatui-widgets 0.3.2 / ratatui-crossterm 0.1.2; Marker::{Braille,Quadrant,Sextant,Octant}, ratatui::run/init/restore | none | verified |
| `crossterm` | 0.29.0 | Backend used by bottom, rmpc, gping; feature osc52 used by rmpc | none | verified |
| `tui-widgets` | 0.7.11 | Umbrella (MIT OR Apache-2.0, MSRV 1.88): tui-bar-graph 0.3.5, tui-equalizer 0.2.3, tui-big-text 0.8.9, tui-scrollview 0.6.7, tui-popup, tui-cards, tui-box-text, tui-qrcode, tui-prompts, tui-scrollbar | none | verified |
| `ratatui-image` | 11.0.6 | Album art: Picker::from_fontsize/from_query_stdio, Protocol::{Halfblocks,Sixel,Kitty,iTerm2}, Image/StatefulImage, ThreadProtocol; MSRV 1.86 | none for Halfblocks; chafa-dyn feature needs libchafa (not installed). Kitty/Sixel unusable in local VTE 0.84 | verified |
| `tachyonfx` | 0.25.1 | Shader-like effects (Effect, Shader, EffectManager, fx::*, dsl) for retrowave transitions/glitch | none | verified |
| `realfft` | 3.5.0 | Pure-Rust real FFT for the spectrum analyser (rustfft 6.4.1 underneath); spectrum-analyzer 1.8.0 as higher-level alternative | none | verified |
| `audioviz` | 0.6.0 | Optional spectrum/distributor helpers (features spectrum, processor) — use WITHOUT the cpal feature; 33% documented | none unless `cpal`/`io` feature (then libasound2-dev) | verified |
| `cpal` | 0.18.2 | Only as an opt-in feature: native capture with optional pipewire/pulseaudio hosts | libasound2-dev ALWAYS on Linux; pipewire feature additionally needs libpipewire-0.3-dev; all absent on this machine (pkg-config checks failed) | verified |
| `zbus` | 5.19.0 | MPRIS (org.mpris.MediaPlayer2.*) client for the Winamp component; ncspot uses zbus for MPRIS | none (pure Rust, no libdbus) | verified |
| `mpris` | 2.1.0 | AVOID: what fum uses; depends on the dbus crate (feature dbus-vendored = [dbus/vendored]) | libdbus-1-dev (absent) or vendored build | verified |
| `nvml-wrapper` | 0.12.1 | GPU metrics/processes — same crate bottom (0.12.1) and zenith (0.10) use; dlopens libnvidia-ml.so.1 | libnvidia-ml.so.1 at runtime (present) | verified |
| `sysinfo` | 0.39.6 | CPU/mem/proc/disk collection; bottom pins =0.39.6, zenith 0.37 | none | verified |
| `tui-logger` | 0.18.3 | In-app log pane widget (bottom/rmpc have log panes) | none | verified |
| `malevich` | 1.20.0 | Optional plotting widget (line/scatter/bar/histogram/heatmap) if ratatui Chart is insufficient | none | likely |
| `tuirealm` | 4.1.0 | Elm/React component framework over ratatui 0.30 (termusic) — evaluated, not recommended | none | verified |
| `viuer` | 0.11.0 | Alternative image printer used by ncspot/termusic (features sixel/icy_sixel); not needed if using ratatui-image | none for halfblocks | verified |
| `scope-tui` | 0.3.5 | Reference app (MIT) for oscilloscope/spectroscope/vectorscope rendering on ratatui 0.29 with rustfft 6.3; cpal/libpulse optional | none for file/pipe input | verified |
| `bottom` | 0.14.9 | Reference only: [lib] exists but docs.rs says internal; start_bottom() sole entry | none | verified |
| `amdgpu_top / libamdgpu_top` | 0.11.5 | Reference only: TUI is cursive 0.21, GPU lib is AMD/libdrm-specific | libdrm (irrelevant on NVIDIA) | verified |
| `astral-watch` | 0.7.0 (local workspace; 0.6.0 cargo-installed) | 12V-2x6 per-pin readings via its public i2c/decode/alert API; its tui feature is ratatui 0.29 (would need bumping to 0.30 to share types) | i2c group membership (user has it); /dev/i2c-* access | verified |

## Risks

- **Album art / pixel graphics: libvte on this host has no Sixel (+BIDI +GNUTLS +ICU +SYSTEMD only) and VTE has no Kitty protocol; Picker::from_query_stdio may stall or mis-detect.** → Default to Picker::from_fontsize((w,h)) → Halfblocks; run stdio query only behind --probe-graphics with a timeout; document kitty/ghostty as optional showcase terminals.
- **No Nerd Fonts; DejaVu/Noto/Liberation Mono may lack Unicode 16 Sextant/Octant 'Symbols for Legacy Computing' glyphs, producing tofu in high-res markers.** → Glyph-set abstraction (braille U+2800 block and Block Elements are covered by DejaVu Sans Mono); test each glyph set on Ptyxis before enabling; never require icon fonts.
- **cpal/pipewire/libpulse crates fail to build (headers absent) if pulled in by a transitive default feature.** → Feature-gate every native audio host; CI job with a clean container proving `cargo build` succeeds with only build-essential + pkg-config; default capture via pw-record/ffmpeg subprocess with `--raw -`.
- **Subprocess capture lifecycle: zombie pw-record on panic, monitor target changes when the default sink switches (Speakers/Headphones/SPDIF), latency (pw-record default 100 ms).** → Own the child with kill-on-drop, select target by parsing `pw-dump`/`wpctl status` for the default sink's monitor, pass `--latency 20ms`, reconnect on EOF.
- **Per-process network (bandwhich-style) needs cap_net_raw/cap_net_admin/cap_sys_ptrace/cap_dac_read_search or sudo; pcap dependencies.** → Ship interface-level stats from /proc/net/dev + /sys/class/net (no privileges) first; per-process as an optional feature with documented setcap.
- **Licence contamination: rav is CC BY-NC-SA 4.0; cava/glava code is C; btop themes are Apache-2.0 (needs NOTICE/attribution if ported).** → Port only formats/ideas, not code, from NC/GPL sources; take spectrum ballistics from webamp (MIT) or reimplement; keep a THIRD_PARTY.md.
- **MSRV/version creep: rmpc master requires 1.97.1, bottom 1.95.0, tui-widgets 1.88; ratatui 0.30's crossterm feature flags (crossterm_0_28/0_29) can cause duplicate-crossterm builds.** → Pin ratatui 0.30.x + crossterm 0.29 workspace-wide; astral-watch tui feature is on ratatui 0.29 — do not link its TUI types, only its data API; MSRV CI job as in astral-watch.
- **Framework/library bit-rot (spotify-tui, sampler, gotop, glava dormant; cursive, tui-rs, tui-realm coupling).** → Depend on ratatui-core for component crates; keep third-party widget crates behind features so any one can be replaced.
- **Config sprawl (btop ~80 keys, htoprc machine-owned) making themes/layouts unreadable.** → Separate files: layout.toml (tree + presets), theme/*.toml (tokens), config.toml (behaviour); write-back only to a dedicated state file, never rewrite user config.

## Verified facts

- amdgpu_top TUI uses cursive 0.21 (crossterm-backend) + termsize, not ratatui — read crates/amdgpu_top_tui/Cargo.toml via raw.githubusercontent; source tree app.rs/lib.rs/smi.rs/view/{activity,fdinfo,gpu_metrics,memory_error_count,perf_counter,sensors,vram,xdna_fdinfo}.rs via gh api git/trees
- amdgpu_top installed locally is 0.11.2 (~/.cargo/.crates.toml); crates.io latest 0.11.5 (cargo info); GitHub release v0.11.5 2026-05-18
- bottom 0.14.9 (2026-08-27) Cargo.toml: [lib] + [[bin]] btm, ratatui 0.30.2, crossterm 0.29.0, ratatui-core 0.1.2, sysinfo =0.39.6, nvml-wrapper 0.12.1, rust-version 1.95.0 — raw Cargo.toml; docs.rs/bottom states docs are for development, entry start_bottom()
- bottom layout TOML ([[row]]/[[row.child]]/ratio/type list) — bottom.pages.dev layout page; internals BottomLayout/BottomRow/BottomCol/BottomColRow/BottomWidget with neighbour Option<u64> ids — raw src/app/layout_manager.rs; widget carousel arrows '◄ name'/'name ►' — raw src/canvas/components/widget_carousel.rs
- bottom [styles] themes Default/Default-light/Gruvbox/Gruvbox-light/Nord/Nord-light and sub-table keys — bottom.pages.dev styling page
- btop v1.4.7 2026-05-01, C++23, Apache-2.0, custom TUI, presets 'box:P:G' format, shown_boxes gpu0-gpu5, graph_symbol braille/block/tty, truecolor/force_tty — GitHub README + releases; dracula.theme has 41 keys (listed) — raw themes/dracula.theme
- htop: local htoprc shows header_layout=two_50_50, column_meters_0/column_meter_modes_0, screen_tabs=1, screen:Main=… and header 'rewritten by htop… parser is also very primitive'; MeterMode.h enum BAR_METERMODE=1, TEXT, GRAPH, LED — gh api contents/MeterMode.h; local htop 3.4.1, latest 3.5.3 2026-08-16
- nvtop local 3.2.0, latest 3.3.2 2026-02-08; man nvtop: F2 setup sections General/Devices/Chart/Processes, F12 saves to $XDG_CONFIG_HOME/nvtop/interface.ini ('Do not edit'), -p max-of-all-GPUs bar, -d delay in tenths, dynamic ENC/DEC meters
- rmpc master Cargo.toml: workspace rmpc/rmpc-mpd/rmpc-shared/rmpcd, ratatui 0.30.0, crossterm 0.29.0 (osc52), image 0.25.9, tokio 1.49.0, rust-version 1.97.1, no ratatui-image dependency; image backends rmpc/src/ui/image/{block,facade,iterm2,kitty,mod,sixel,ueberzug}.rs and AlbumArtFacade/ImageBackend enum with show/display/hide/cleanup/set_size — raw facade.rs + gh api tree; rmpc-mpd on crates.io is 0.0.0 placeholder (cargo search)
- rmpc default layout RON (Header '2', Tabs '3', TabContent '100%', ProgressBar '1'), Component("name") reuse, sizes as cells or percent — rmpc.mierak.dev/configuration/layout; Cava pane spawns cava over MPD FIFO with bar_symbols/inverted_bar_symbols/bar_width/bar_spacing/orientation/bar_color — rmpc.mierak.dev/configuration/cava
- VTE on this host: strings on /usr/lib/x86_64-linux-gnu/libvte-2.91-gtk4.so.0 and libvte-2.91.so.0 contain feature string '+BIDI +GNUTLS +ICU +SYSTEMD' (no +SIXEL); no kitty/ghostty/wezterm/foot/alacritty/ueberzugpp/chafa/timg/viu binaries found
- pkg-config --exists alsa / libpipewire-0.3 / libpulse all fail locally; /usr/include/alsa absent; cpal README states libasound2-dev is needed even with JACK/PipeWire/PulseAudio features; cpal 0.18.2 features pipewire=[dep:pipewire], pulseaudio=[dep:pulseaudio…] (cargo info)
- pw-record --help exposes --target, --latency (default 100ms), -P/--properties, --rate (48000), --channels (2), --format (s16), -a/--raw; pw-cat compiled with libpipewire 1.6.2; `ffmpeg -sources pulse` lists the USB DAC Headphones sink monitor as default (*) plus Speakers/SPDIF/HDMI monitors
- mpris 2.1.0 has feature dbus-vendored=[dbus/vendored] (libdbus dependency); fum's Cargo.toml uses mpris 2.0.1, ratatui 0.29, ratatui-image 4.1.0, crossterm 0.28.1 — gh api contents
- GitHub metadata sweep (language/license/pushed_at/archived/stars/latest release) for 42 repos on 2026-08-30 via gh api — e.g. ytop archived 2020-08-29; sampler last release 2019-12-24; spotify-tui last release 2021-08-24; glava last release 2019; bandwhich v0.23.1 2024-10-08; trippy 0.13.0 2025-05-05; cava 1.0.0 2026-06-13; ncspot v1.4.0 2026-08-21; termusic v0.13.2; zenith 0.15.0 2026-05-08; wtfutil v0.50.0; termdash v0.20.0; cool-retro-term 2.0.0-beta2 2026-05-31; cliamp v1.63.2
- tui-widgets 0.7.11 features/sub-crates (bar-graph, big-text, box-text, cards, equalizer, popup, prompts, qrcode, scrollbar, scrollview), MSRV 1.88, MIT OR Apache-2.0 — cargo info; tui-equalizer 0.2.3 Equalizer{bands, brightness} and tui-bar-graph 0.3.5 builder API — docs.rs; tui-big-text 0.8.9 PixelSize variants — docs.rs
- ratatui-image 11.0.6 (MIT, MSRV 1.86) API: Picker::from_query_stdio, Protocol Halfblocks/Sixel/Kitty/iTerm2, Image/StatefulImage, Resize::Fit/Crop/Scale, ThreadProtocol; features crossterm/chafa-dyn/chafa-static — cargo info + docs.rs
- gotop layout DSL '(rowspan:)?widget(/weight)?', default layout '2:cpu / disk/1 2:mem/2 / temp / 2:net 2:procs', kitchensink with power — raw docs/layouts.md + gh api contents layouts/default, layouts/kitchensink; sampler go.mod uses gizak/termui/v3 v3.0.0
- wtfutil sample_config.yml: grid columns [35,35,35,35], rows [10,10,10,10,4], refreshInterval, colors.border.{focusable,focused,normal}, module position top/left/height/width — gh api contents _sample_configs/sample_config.yml
- termdash widgets directory listing (barchart, borderfx, button, checkbox, donut, dropdown, fx, gauge, heatmap, linechart, modal, pie, radar, radio, segmentdisplay, slider, sparkline, spectrum, spinner, tab, text, textinput, threed, timeline, toast, treeview) — gh api git/trees
- scope-tui 0.3.5 Cargo.toml: rustfft 6.3, cpal 0.15 optional, ratatui 0.29 optional, crossterm 0.29 optional, libpulse-binding/libpulse-simple-binding optional — gh api contents
- zenith Cargo.toml: ratatui 0.29.*, crossterm 0.28.*, sysinfo 0.37, nvml-wrapper 0.10.0 optional — gh api contents
- trippy sample config [theme-colors] key list and [bindings] examples, tui-refresh-rate — raw trippy-config-sample.toml
- Winamp VISCOLOR.TXT 24-row semantics (0 bg, 1 dots, 2-17 spectrum peak→base, 18-22 oscilloscope, 23 peak marker) and PLEDIT.TXT keys — winampskins.neocities.org/config
- astral-watch src/tui.rs: struct Theme, App, CardTab, run_tui, spawn_gpu_poller (nvidia-smi subprocess), draw_* functions; Cargo.toml ratatui = 0.29 optional behind `tui` feature (MSRV 1.88) — local grep
- cli-visualizer original repo dpayne/cli-visualizer returns 404 (author deleted account); mirrors PosixAlchemist/cli-visualizer and 69keks/cli-visualizer-git exist — gh api + web search

## Open questions

- Is Ptyxis/VTE the target showcase terminal for the long term? If so, all art/visualizers must be designed for halfblocks + braille/block glyphs; if a Kitty-protocol terminal (kitty/ghostty/wezterm) may be installed later, ratatui-image's Kitty path should be kept behind a probe flag.
- Audio capture: is a runtime dependency on pw-record/pw-cat (pipewire-utils) acceptable as the default, with cpal as an opt-in feature requiring libasound2-dev, or should the user install libasound2-dev now to allow native cpal+pipewire capture?
- Should the 12V-2x6 pin component link astral_watch directly (direct i2c reads, needs i2c group — present) or scrape its Prometheus exporter on 127.0.0.1:9942 to avoid bus contention if the astral-watch service is ever installed?
- Is per-process network accounting (bandwhich-style, needs cap_net_raw/cap_net_admin/cap_sys_ptrace/cap_dac_read_search) in scope, or is per-interface + per-connection-from-/proc enough?
- Which layout config syntax does the user prefer: bottom-style nested TOML tables, or a compact gotop-style grid DSL string inside TOML (e.g. `rows = ["2:gpu/2 pins/1", "cpu net audio"]`)?
- Do Unicode 16 Sextant/Octant glyphs render in DejaVu Sans Mono / Noto Sans Mono on this box? (Needs an interactive terminal test; not checkable from this subagent.)
- Should astral-watch's TUI be upgraded to ratatui 0.30 so opsTui can share widget code, or should opsTui only consume the data API (i2c/decode/alert) and reimplement the bars?
- Winamp component fidelity: emulate the classic 275x116 main window (time digits via tui-big-text, 16-row VISCOLOR spectrum, peak caps) as a fixed-footprint panel, or a flexible MPRIS 'now playing' widget like fum with an optional classic skin footprint?

## Sources

- https://github.com/aristocratos/btop
- https://github.com/aristocratos/btop/releases
- https://raw.githubusercontent.com/aristocratos/btop/main/themes/dracula.theme
- https://raw.githubusercontent.com/aristocratos/btop/main/README.md
- https://github.com/ClementTsang/bottom
- https://github.com/ClementTsang/bottom/releases
- https://raw.githubusercontent.com/ClementTsang/bottom/main/Cargo.toml
- https://raw.githubusercontent.com/ClementTsang/bottom/main/src/app/layout_manager.rs
- https://raw.githubusercontent.com/ClementTsang/bottom/main/src/canvas/components/widget_carousel.rs
- https://bottom.pages.dev/stable/configuration/config-file/layout/
- https://bottom.pages.dev/stable/configuration/config-file/styling/
- https://docs.rs/bottom/latest/bottom/
- https://github.com/bvaisvil/zenith
- https://github.com/xxxserxxx/gotop
- https://raw.githubusercontent.com/xxxserxxx/gotop/master/docs/layouts.md
- https://github.com/cjbassi/ytop
- https://github.com/nicolargo/glances
- https://github.com/aristocratos/bpytop
- https://github.com/amanusk/s-tui
- https://nmon.sourceforge.io/pmwiki.php?n=Main.HomePage
- https://github.com/htop-dev/htop (Meter.h, MeterMode.h via API)
- https://github.com/Syllo/nvtop
- https://github.com/XuehaiPan/nvitop
- https://github.com/wookayin/gpustat
- https://github.com/Umio-Yasuno/amdgpu_top
- https://raw.githubusercontent.com/Umio-Yasuno/amdgpu_top/main/crates/amdgpu_top_tui/Cargo.toml
- https://raw.githubusercontent.com/Umio-Yasuno/amdgpu_top/main/crates/amdgpu_top_tui/src/app.rs
- https://raw.githubusercontent.com/Umio-Yasuno/amdgpu_top/main/crates/amdgpu_top_tui/src/smi.rs
- https://github.com/ulissesf/qmassa
- https://github.com/wtfutil/wtf
- https://github.com/wtfutil/wtf/blob/master/_sample_configs/sample_config.yml
- https://github.com/sqshq/sampler
- https://github.com/mum4k/termdash
- https://github.com/davidbrochart/jpterm
- https://github.com/nschloe/tiptop
- https://github.com/Textualize/textual
- https://github.com/imsnif/bandwhich
- https://github.com/GyulyVGC/sniffnet
- https://github.com/fujiapple852/trippy
- https://raw.githubusercontent.com/fujiapple852/trippy/master/trippy-config-sample.toml
- https://github.com/orf/gping
- https://github.com/tgraf/bmon
- https://github.com/karlstav/cava
- https://github.com/PosixAlchemist/cli-visualizer
- https://github.com/69keks/cli-visualizer-git
- https://github.com/jarcode-foss/glava
- https://docs.rs/audioviz/latest/audioviz/
- https://github.com/alemidev/scope-tui
- https://github.com/i-am-logger/rav
- https://github.com/RustAudio/cpal
- https://github.com/hrkfdn/ncspot
- https://github.com/Rigellute/spotify-tui
- https://github.com/mierak/rmpc
- https://raw.githubusercontent.com/mierak/rmpc/master/Cargo.toml
- https://raw.githubusercontent.com/mierak/rmpc/master/rmpc/src/ui/image/facade.rs
- https://rmpc.mierak.dev/configuration/layout/
- https://rmpc.mierak.dev/configuration/theme/
- https://rmpc.mierak.dev/configuration/cava/
- https://rmpc.mierak.dev/configuration/album-art/
- https://github.com/tramhao/termusic
- https://github.com/clangen/musikcube
- https://github.com/cmus/cmus
- https://github.com/qxb3/fum
- https://raw.githubusercontent.com/qxb3/fum/main/doc-site/docs-content/02_configuring/doc.md
- https://github.com/bjarneo/cliamp
- https://github.com/captbaritone/webamp
- https://winampskins.neocities.org/config
- https://github.com/Swordfish90/cool-retro-term
- https://github.com/ratatui/awesome-ratatui
- https://ratatui.rs/highlights/v030/
- https://docs.rs/ratatui-image/latest/ratatui_image/
- https://docs.rs/tui-equalizer/latest/tui_equalizer/
- https://docs.rs/tui-bar-graph/latest/tui_bar_graph/
- https://docs.rs/tui-big-text/latest/tui_big_text/
- https://docs.rs/tachyonfx/latest/tachyonfx/
- https://docs.rs/tuirealm/latest/tuirealm/
- https://www.arewesixelyet.com/
- local: cargo info/cargo search on crates.io (2026-08-30); gh api repos/* metadata; ~/.config/htop/htoprc; man nvtop; man htop; pw-record --help; ffmpeg -sources pulse; strings libvte-2.91*.so.0; pkg-config; /home/mattbeam/workspace/astral-watch/src/tui.rs and Cargo.toml
