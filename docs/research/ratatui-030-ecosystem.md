<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# ratatui 0.30 and its ecosystem as the rendering foundation for opsTui (0.29→0.30 changes, rendering primitives, frame loop/perf, event handling on VTE/Ptyxis, testing, skeleton)

# ratatui 0.30.x as the rendering foundation

## 1. What changed 0.29 → 0.30 (and what it means for porting astral-watch's `tui.rs`)

**Releases** (CHANGELOG): 0.30.0 = 2025-12-26, 0.30.1 = 2026-06-05, 0.30.2 = 2026-06-19. `cargo info ratatui@0.30.2` reports **rust-version 1.88.0** (the 0.30.0 notes say 1.86; the published 0.30.2 metadata says 1.88 — use 1.88 for the MSRV CI job). Edition 2024.

**Workspace split.** `ratatui` is now a facade over `ratatui-core 0.1.2` (Buffer/Cell, Rect/Layout, Style/Color, text, `Widget`/`StatefulWidget`, `Terminal`, `TestBackend`), `ratatui-widgets 0.3.2` (Block, BarChart, Canvas, Chart, Gauge/LineGauge, List, Paragraph, Scrollbar, Sparkline, Table, Tabs, Clear, Fill, RatatuiLogo, RatatuiMascot, Monthly), `ratatui-crossterm 0.1.2` (default feature `crossterm_0_29`; `crossterm_0_28` selectable), `ratatui-macros 0.7.2` (re-exported via the default `macros` feature), plus `ratatui-termion`, `ratatui-termwiz`, and new in 0.30.2 `ratatui-termina 0.1.0`. Rule: **apps depend on `ratatui`; widget libraries depend on `ratatui-core`** (tachyonfx, tui-big-text, tui-equalizer already do). Default features of `ratatui 0.30.2`: `all-widgets, crossterm, layout-cache, macros, underline-color`; optional `palette`, `serde`, `scrolling-regions`, `unstable-widget-ref`.

**Breaking changes that can bite** (BREAKING-CHANGES.md):
- `Marker` is `#[non_exhaustive]` (new `Quadrant`, `Sextant`, `Octant`) — exhaustive matches need `_`.
- `Flex::SpaceAround` now mirrors CSS; old behaviour is `Flex::SpaceEvenly`.
- `widgets::block::Title` removed: `Block::title(impl Into<Line>)`, `title_top/title_bottom`, `title_position(TitlePosition)`; `BlockExt` moved to `widgets::BlockExt`.
- `Style` no longer implements `Styled`; shorthand (`Style::new().red().on_black()`) are now **const inherent methods** — a `use ratatui::style::Stylize;` kept only for `Style` becomes an unused import (fails `-D warnings`).
- `layout::Alignment` → `HorizontalAlignment` (alias kept), `VerticalAlignment` added.
- Backend trait: associated `Error` type, `clear_region` required; `TestBackend::Error = Infallible`; crossterm↔ratatui conversions via `FromCrossterm`/`IntoCrossterm` (no more `From`).
- `WidgetRef`/`StatefulWidgetRef` stay behind `unstable-widget-ref`; blanket impl reversed (`impl<W: Widget> WidgetRef for &W`); `Frame::render_widget_ref` needs `FrameExt`.
- `layout-cache` is a feature (on by default); `Layout::init_cache` only with it.

**New APIs worth using:** `ratatui::run(|terminal: &mut DefaultTerminal| …)` (init + restore + panic hook; verified in `ratatui/src/init.rs`: raw mode + alternate screen only, no mouse/paste/keyboard-enhancement, no BufWriter, `DefaultTerminal = Terminal<CrosstermBackend<Stdout>>`), `Rect::layout::<N>()/try_layout/layout_vec`, `Rect::centered(Constraint, Constraint)`, `centered_horizontally/vertically`, `Rect::outer`, `Rect::resize`, `Position::offset`, `Layout::try_areas`, `Block::merge_borders(MergeStrategy::{Replace,Exact,Fuzzy})` (auto-joins `│`+`─`→`┼` for adjacent panes), six dashed `BorderType`s, `Block::shadow(Shadow)` (0.30.1: `Shadow::overlay()/block()/light_shade()/medium_shade()/dark_shade()/symbol(&str)/custom(effect)`, `.style()`, `.offset(Offset)`), `CellDiffOption::{AlwaysUpdate, ForceWidth(NonZeroU16), Skip}` replacing the deprecated `Cell::skip`, `Terminal::apply_buffer[_with_cursor]` (0.30.2; flush without a draw closure), `Text += `, `Color::from([r,g,b])`, `Style::has_modifier`, serde for layout/style types, `BarChart::horizontal/grouped`, `Bar::with_label`, `LineGauge::filled_symbol/unfilled_symbol`, `Tabs::width`, `ScrollbarState::get_position`, `Sparkline` per-bar `SparklineBar` styles (since 0.29).

**Porting `astral-watch/src/tui.rs` (1695 lines, ratatui 0.29):** grep shows it uses `ratatui::init()/restore()`, `ratatui::crossterm::{event::poll/read, execute!, EnableMouseCapture}`, `Layout::vertical/horizontal(...).areas()/split()`, `Flex::Center`, `Constraint::{Length,Min,Percentage}`, `buf.cell(Position{..})`/`buf.cell_mut((x,y))`/`Cell::set_style`, `symbols::Marker::Braille`, `Alignment::Center`, `Block::title(&str)`, `BorderType::Rounded`, `Sparkline`, `Chart/Axis/Dataset`, `Gauge`, `List`, `Scrollbar`, `Clear`, `Paragraph`, and tests via `Terminal::new(TestBackend::new(w,h))` + `backend().buffer().content`. **All of those still exist unchanged in 0.30.2** (`Buffer.area/content` are still `pub`; `cell/cell_mut` take `impl Into<Position>`). No `block::Title`, no `Stylize` import, no `Flex::SpaceAround`, no exhaustive `Marker` match → the port is essentially `ratatui = "0.30"` + MSRV 1.88 for the `tui` feature. Only real polish item: prefer `impl Widget for &App` over free `draw(f, &app)` functions so the same code works for opsTui components and TestBackend snapshots.

## 2. Rendering primitives for a showcase look

- **Cells/Buffer:** `Cell { fg, bg, underline_color, modifier, diff_option }`, symbol is a `CompactString` (`set_symbol/set_char/merge_symbol/set_fg/set_bg/set_style/reset`, `Cell::EMPTY`). `Buffer::{empty, filled, with_lines, cell, cell_mut, index_of, pos_of, set_string, set_stringn, set_line, set_span, set_style(area), merge, diff, diff_iter}`; `buf[(x,y)]` indexing. Custom painting is just writing cells, exactly like astral-watch's `set_cell/put_centered` helpers.
- **Colors:** `Color::Rgb(r,g,b)`, `Indexed(u8)`, `from_u32(0xRRGGBB)`, `FromStr` (`"#ff5500"`, names), `From<[u8;3]>/(u8,u8,u8)`; with the `palette` feature `Color::from_hsl/from_hsluv` and `From<palette::Srgb>` — gradients = interpolate in `palette` (0.7.7) per cell. `Modifier::{BOLD,DIM,ITALIC,UNDERLINED,SLOW_BLINK,RAPID_BLINK,REVERSED,HIDDEN,CROSSED_OUT}`; underline colour via `underline-color` (VTE supports SGR 58). Terminal here is truecolor (`COLORTERM=truecolor`).
- **Symbols:** `symbols::border::{PLAIN,ROUNDED,DOUBLE,THICK,LIGHT/HEAVY_{DOUBLE,TRIPLE,QUADRUPLE}_DASHED,QUADRANT_INSIDE,QUADRANT_OUTSIDE,ONE_EIGHTH_WIDE/TALL,PROPORTIONAL_WIDE/TALL,FULL,EMPTY}` (+ custom `border::Set`), `bar::{NINE_LEVELS,THREE_LEVELS}`, `block`, `shade`, `braille`, `half_block`, `pixel` (quadrant/sextant/octant), `scrollbar`, `merge::MergeStrategy`. `BorderType` has 12 variants.
- **Charts:** `Sparkline` (per-bar `SparklineBar{value: Option<u64>, style}`, `max`, `direction(RenderDirection::RightToLeft)`, `absent_value_symbol`), `BarChart` (`vertical/horizontal/grouped`, `bar_width/bar_gap/group_gap/bar_set`, per-`Bar` `style/value_style/text_value`), `Chart` (`Dataset::{data(&[(f64,f64)]), marker, graph_type(Line|Scatter|Bar), style}`, `Axis::{bounds, labels, labels_alignment}`, `LegendPosition`, `hidden_legend_constraints`), `Gauge`/`LineGauge`.
- **Canvas** (`widgets::canvas`): `Canvas::default().x_bounds().y_bounds().marker(Marker).paint(|ctx| { ctx.draw(&Line/Points/Rectangle/Circle/Map); ctx.layer(); ctx.print(x,y,line) })`; custom `Shape { fn draw(&self, painter: &mut Painter) }`; `Painter::{get_point, paint(x,y,color)}`. Sub-cell resolutions: **Braille 2×4, Octant 2×4, Sextant 2×3, Quadrant 2×2, HalfBlock 1×2, Block/Bar/Dot 1×1**, per-point colour (gradient scopes/waveforms).
- **Glyph availability on this box (verified):** VTE 0.84.0's `src/minifont.cc` draws box drawing U+2500–257F, block elements U+2580–259F, sextants U+1FB00–1FB3B and **octants U+1CD00–1CDE5 natively** (no font needed), so `Marker::Octant/Sextant` and `PixelSize::Octant` work even though `fc-list` shows no installed font with U+1CD00. Braille U+2800 is *not* in minifont; fontconfig falls back from DejaVu Sans Mono to DejaVu Sans / Noto Sans Symbols2 (both cover U+2800). No Nerd Font: U+E0B0 powerline glyphs exist only in OpenSymbol — do not use private-use icons.
- **tui-big-text 0.8.9** (depends on ratatui-core; MSRV 1.88): `BigText::builder().pixel_size(PixelSize::{Full,HalfHeight,HalfWidth,Quadrant,ThirdHeight,Sextant,QuarterHeight,Octant}).style(..).lines(vec![Line…]).centered().build()`, font8x8 glyphs → large digits/clocks/Winamp-style readouts (Octant = 8 px in one cell height).
- **tui-widgets 0.7.11** umbrella (MSRV 1.88; sub-crates also standalone): `tui-bar-graph 0.3.5` (Braille/Solid bars with `colorgrad` gradients, `ColorMode::VerticalGradient` — ideal for spectrum bars), `tui-equalizer 0.2.3` (`Equalizer{bands: Vec<Band>, brightness}`, 0.0–1.0 bands), `tui-popup 0.7.6`, `tui-scrollview 0.6.7`, `tui-cards`, `tui-box-text`, `tui-prompts`, `tui-qrcode`, `tui-scrollbar`. `throbber-widgets-tui 0.11.1`: `Throbber` + `ThrobberState::calc_next()` with ~20 symbol sets (BRAILLE_*, CLOCK, QUADRANT_BLOCK, …), ratatui ^0.30.
- **tachyonfx 0.25.1** (depends on `ratatui-core ^0.1.2`; default features `std, dsl`; `sendable`, `std-duration`, `wasm` optional): post-processing "shaders" over already-rendered cells. `EffectRenderer<T>` is implemented for `Frame` and `Buffer`: `frame.render_effect(&mut effect, area, last_tick: tachyonfx::Duration)` where `Duration` is a custom `{ milliseconds: u32 }` (swap to std with `std-duration`). Build with `fx::*`: colour (`fade_from/to[_fg]`, `hsl_shift[_fg]`, `paint`, `darken/lighten/saturate`, `term256_colors`), text (`dissolve/coalesce`, `evolve`, `explode`, `sweep_in/out`, `slide_in/out`), geometry (`translate`, `expand`, `stretch`), control (`sequence`, `parallel`, `repeat/repeating`, `ping_pong`, `delay`, `sleep`, `never_complete`, `prolong_start/end`, `freeze_at`, `with_duration`), custom (`effect_fn`, `effect_fn_buf`, `offscreen_buffer`, `dynamic_area`). Timing via `EffectTimer::from_ms(u32, Interpolation)` or `(500, Interpolation::SineOut).into()`. Targeting via `Effect::with_area(Rect)`, `with_filter(CellFilter::{Text, FgColor(c), Outer(margin), AllOf(..)…})`, `with_pattern`, `with_color_space(ColorSpace)`, `reversed()`, `running()/done()`. `EffectManager` keys effects by id for replace/cancel. `Effect` is `!Send` unless `sendable` — keep effects on the render thread. The `dsl` feature (`EffectDsl::new().compiler().compile("fx::sequence(&[fx::dissolve(300), fx::fade_to_fg(Color::Red, 500)])")`) lets themes ship effects as strings (hot-reloadable retrowave glitch/scanline effects). Cost is O(cells in the effect area) per frame, applied after widgets, so it composes trivially: render layout → run per-component effects → run global theme effect.
- **ratatui-image 11.0.6** (rust-version 1.86): protocols `Halfblocks` (▀/▄ with fg/bg, 1 cell = 1×2 px), `Sixel` (pure-Rust `icy_sixel`), `Kitty`, `Iterm2`; `Picker::from_query_stdio()` (DA1 / cell-size queries, call after entering alt screen, before reading events) or `Picker::halfblocks()`; `Image` (static) / `StatefulImage` + `ThreadProtocol` for background resizing; `Resize::{Fit,Crop,Scale}`. **Default features include `chafa-dyn`, which needs libchafa via pkg-config at build and `libchafa.so` at runtime — neither is installed here** → use `default-features = false, features = ["image-defaults", "crossterm"]`. **Sixel will not work in this Ptyxis:** VTE's `meson_options.txt` has `sixel` default `false`, Ubuntu's `debian/rules` passes no `-Dsixel`, Debian #1059446 closed *wontfix* (2025-07-14, "upstream does not think it is ready"), and `objdump -T /usr/bin/ptyxis` imports no `vte_terminal_set_enable_sixel`. Expect halfblocks (e.g. album art at 2 px per cell — legible but coarse); kitty protocol is also absent on VTE.

## 3. Frame loop, events, throughput

**Pipeline** (`Terminal::draw`): autoresize (queries backend size each frame, so `Event::Resize` needs no handling) → fresh `Frame` over the back buffer → your closure renders widgets → `Buffer::diff` old vs new (cells compared as CompactString + style; wide-char and `CellDiffOption` aware) → `CrosstermBackend::draw` queues `MoveTo` only for non-contiguous runs and emits style deltas via `ModifierDiff` → single `flush()` → swap buffers. Immediate mode: everything not rendered is cleared. Render order matters (later widgets overwrite).

**Measured numbers from upstream:** GitHub discussion #579 — a 200×50 screen where *every* cell changes fg+bg each frame ran ~3 fps initially and **24–40 fps on an M2 Max after optimisation, with 98% of profile samples in the `write` syscall** (stdout ~2–3× faster than unbuffered stderr, 11–12 fps); synchronized-output made no measurable difference there. Issue #1338 — a template drawing *static* content at 60 fps cost ~7% of one core in release (50% in debug); the flamegraph is the diff pass, and kdheepak notes the diff stops paying off once >30–40% of cells change. Implications for 250×70 (17,500 cells): the diff itself is ~tens–hundreds of µs per frame in release; what limits you is bytes emitted per frame × the terminal's parsing/painting. A spectrum analyser that touches, say, a 120×30 region with per-cell RGB is ~3.6k changed cells/frame → comfortably 30 fps and plausibly 60 fps on VTE, but **only a measurement in Ptyxis will confirm**; design for a configurable 30 fps default, 60 fps opt-in, and a `dirty`/"draw only when state changed" gate for the non-animated panels. Cheaper cells help: reuse the same `Style` across runs (fewer SGR sequences), avoid re-randomising backgrounds, keep effects area-scoped. `BeginSynchronizedUpdate/EndSynchronizedUpdate` exist in crossterm 0.29 and are harmless to send, but VTE/GNOME Terminal report DEC 2026 as permanently disabled (DECRPM 4), so expect no tearing benefit on this machine.

**Events (crossterm 0.29):** `event::poll(Duration)` + `event::read()` (never from two threads), or `EventStream` (`event-stream` feature, `futures::Stream<Item = io::Result<Event>>`). `Event::{Key, Mouse, Resize(u16,u16), FocusGained, FocusLost, Paste}` with 0.29's helpers `is_key_press()`, `as_key_press_event()`, `as_mouse_event()`, `as_resize_event()`; `KeyEvent{code, modifiers, kind: Press|Release|Repeat, state}`. Mouse (SGR) works on VTE — enable with `execute!(stdout(), EnableMouseCapture)` (not done by `ratatui::init`, and you must `DisableMouseCapture` in your own panic hook like astral-watch does). **Kitty keyboard protocol: not available on VTE 0.84** — GNOME MR !14 "kitty keyboard protocol implementation" is still `opened` (created 2025-12-14, updated 2026-04-11, not merged); so no key-release events, no Ctrl+I vs Tab, etc.; gate optional enhancements on `crossterm::terminal::supports_keyboard_enhancement()`. Bracketed paste and focus events are supported by VTE.

**tokio vs std threads — community stance:** the ratatui FAQ recommends the single-threaded `Get event → update → render` loop unless other parts of the app need async ("the only part of ratatui that benefits from async is reading key events"). Official templates cover both (`simple`, `event-driven` with a std thread + mpsc, and `simple-async`/`event-driven-async` with tokio + `EventStream` + `tokio::select!`). For opsTui — collectors that are inherently blocking (procfs parsing, NVML polling, astral-watch i2c reads, PipeWire capture callbacks, FFT) — std threads + one `mpsc`/`crossbeam` channel into a render thread that owns the `Terminal` is the simplest robust design; only zbus (MPRIS) is async-native, and it has a `zbus::blocking` API or can run on a private single-thread tokio runtime inside its collector thread. tachyonfx effects being `!Send` reinforces "render thread owns UI state".

## 4. Testing

`ratatui::backend::TestBackend::new(w,h)` (Error = `Infallible`), `Terminal::new(backend)`, `terminal.draw(|f| f.render_widget(&app, f.area()))`, then `backend().buffer()` (pub `content: Vec<Cell>`, `area`), `assert_buffer_lines([...])`, `assert_cursor_position`, `scrollback()`. `TestBackend: Display` prints the buffer as text rows, so `insta::assert_snapshot!(terminal.backend())` (insta 1.48.0, `cargo insta review`) gives golden screenshots per footprint (6×3, 4×2, 1×1 variants of every component are exactly the kind of matrix snapshot tests shine at). Styles/colours are **not** part of the snapshot; assert colours by inspecting `buffer.cell((x,y)).unwrap().fg`. Widgets implemented as `impl Widget for &Component` can also be rendered straight into `Buffer::empty(Rect::new(0,0,w,h))` without a Terminal.

## 5. Minimal skeleton (checked name-by-name against docs.rs for 0.30.2 / crossterm 0.29; not compiled here because this task was read-only)

```toml
[package]
name = "opstui"
edition = "2024"
rust-version = "1.88"

[dependencies]
ratatui   = "0.30.2"      # defaults: crossterm 0.29 backend, macros, layout-cache, underline-color
color-eyre = "0.6.5"
# later arcs: tachyonfx = "0.25.1", tui-big-text = "0.8.9", tui-bar-graph = "0.3.5",
#             ratatui-image = { version = "11.0.6", default-features = false, features = ["image-defaults", "crossterm"] }

[dev-dependencies]
insta = "1.48"
```

```rust
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Sparkline, Widget};
use ratatui::DefaultTerminal;

enum Msg { Input(Event), Sample(Vec<u64>) }

struct App { hist: Vec<u64>, quit: bool }

impl Widget for &App {                      // 0.30 convention: render by reference
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [top, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
        Line::from("opsTui").centered().render(top, buf);
        Sparkline::default()
            .block(Block::bordered().border_type(BorderType::Rounded).title("cpu"))
            .data(&self.hist)                // IntoIterator<Item: Into<SparklineBar>>; &u64 works
            .style(Style::new().fg(Color::Rgb(255, 0, 128)))
            .render(body, buf);
    }
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let (tx, rx) = mpsc::channel::<Msg>();
    let tx_in = tx.clone();
    thread::spawn(move || while let Ok(ev) = event::read() {      // sole reader of stdin events
        if tx_in.send(Msg::Input(ev)).is_err() { break; }
    });
    thread::spawn(move || loop {                                  // a collector
        thread::sleep(Duration::from_millis(500));
        if tx.send(Msg::Sample(vec![1, 4, 2, 8])).is_err() { break; }
    });
    ratatui::run(|terminal| run(terminal, rx))                    // init/raw/alt-screen/panic hook/restore
}

fn run(terminal: &mut DefaultTerminal, rx: mpsc::Receiver<Msg>) -> color_eyre::Result<()> {
    let mut app = App { hist: Vec::new(), quit: false };
    let frame = Duration::from_millis(33);                         // ~30 fps; make configurable
    let mut next = Instant::now();
    while !app.quit {
        terminal.draw(|f| f.render_widget(&app, f.area()))?;       // autoresize handles Resize
        next += frame;
        loop {
            let now = Instant::now();
            if now >= next { break; }
            match rx.recv_timeout(next - now) {
                Ok(Msg::Input(Event::Key(k))) if k.kind == KeyEventKind::Press => {
                    if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) { app.quit = true; }
                }
                Ok(Msg::Input(_)) => {}
                Ok(Msg::Sample(v)) => app.hist.extend(v),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => app.quit = true,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    #[test]
    fn renders_sparkline() {
        let app = App { hist: vec![1, 2, 3], quit: false };
        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| f.render_widget(&app, f.area())).unwrap();
        insta::assert_snapshot!(t.backend());
    }
}
```

Notes: `ratatui::run` is `fn run<F, R>(f: F) -> R where F: FnOnce(&mut DefaultTerminal) -> R`; `terminal.draw` returns `io::Result<CompletedFrame>` for the crossterm backend; mouse capture and `EnableFocusChange` must be added by you (and undone in a chained panic hook). Add `--fps`, `--no-effects` and a frame-time overlay early so VTE throughput can be measured on the real 250×70 layout.

## Recommendations

- **Build on ratatui 0.30.2 with the default crossterm 0.29 backend; set rust-version = "1.88", edition 2024; use ratatui::crossterm re-export instead of a separate crossterm dependency (add crossterm only if you want the event-stream feature).** — 0.30.2 is the current stable line (2026-06-19), the crate metadata declares MSRV 1.88, and using the re-export guarantees crossterm type unification with ratatui-crossterm's crossterm_0_29 dependency.
  - alternatives: ratatui-termina 0.1.0 backend (new in 0.30.2) or termwiz; not needed on Linux/VTE.
- **Implement every component as `impl Widget for &Component` (plus `StatefulWidget` where scroll state exists) rendering into a `Rect` footprint; keep `WidgetRef` (unstable-widget-ref) out of the codebase.** — This is the documented 0.30 convention, keeps components reusable across frames and testable via TestBackend/Buffer, and avoids the unstable feature whose blanket impl direction already flipped in 0.30.
  - alternatives: Free `fn draw(frame, &state)` functions as in astral-watch's tui.rs — works but is harder to compose in a grid and to snapshot per footprint.
- **Use std threads + an mpsc/crossbeam channel: one input thread blocked in `event::read()`, one thread per collector, and a render thread that owns the Terminal, App state and tachyonfx effects; fixed frame budget (30 fps default, 60 fps opt-in) with a dirty flag for static panels.** — Matches the ratatui FAQ guidance (async only pays for stdin reads), all planned collectors are blocking (procfs, NVML, i2c, PipeWire, FFT), tachyonfx effects are !Send by default, and upstream measurements show the terminal write path, not the diff, is the bottleneck.
  - alternatives: tokio + crossterm EventStream + tokio::select! (official simple-async/event-driven-async templates) — reasonable if zbus/MPRIS ends up needing a shared runtime; can still be confined to the MPRIS collector thread.
- **Adopt tachyonfx 0.25.1 as the theme effects layer (scoped per-component effects + one global theme effect), with the `dsl` feature so themes can carry effect strings; start with `sendable` off.** — It depends only on ratatui-core 0.1.2, composes as a post-pass over the rendered Buffer/Frame, has area/cell filters and an EffectManager, and its cost scales with the affected area — ideal for retrowave glitch/scanline/fade-in without touching widget code.
  - alternatives: Hand-written Buffer post-processing (simple colour ramps) — cheaper but you would re-implement timers/easing/composition.
- **Treat graphics as halfblocks-only on this workstation: if ratatui-image is used, disable default features (`default-features = false, features = ["image-defaults", "crossterm"]`) and call `Picker::from_query_stdio()` with a halfblocks fallback; do not design components that require Sixel/Kitty.** — Verified: Ubuntu's libvte 0.84.0-2 is built without sixel (meson default false, no -Dsixel in debian/rules, Debian #1059446 wontfix), Ptyxis 50.1 imports no vte_terminal_set_enable_sixel, and libchafa (required by the default chafa-dyn feature) is not installed.
  - alternatives: Ship without ratatui-image entirely and draw album art/logos with Canvas markers (Octant/Sextant/Braille) which VTE 0.84 renders natively.
- **Design keybindings for the legacy keyboard encoding (no key-release, no Ctrl+I/Tab distinction), and gate optional enhancements on crossterm::terminal::supports_keyboard_enhancement(); enable SGR mouse capture and undo it in a chained panic hook.** — VTE MR !14 (kitty keyboard protocol) is still open/unmerged as of 2026-04-11, so VTE 0.84 in Ptyxis lacks it; mouse, bracketed paste and focus events do work.
  - alternatives: Require a kitty-protocol terminal (kitty/foot/wezterm/ghostty) — contradicts the user's Ptyxis setup.
- **Use TestBackend + insta snapshots as the primary widget test strategy, one snapshot per component per footprint (e.g. 6x3, 4x2, 1x1), plus targeted `buffer.cell((x,y)).fg` assertions for theme colours.** — TestBackend implements Display (symbols only), Error = Infallible, and insta 1.48 makes golden screens cheap to review; colours are not in the snapshot text so they need explicit cell assertions.
  - alternatives: Manual `assert_buffer_lines` arrays (verbose) or no rendering tests.
- **When porting astral-watch's tui.rs, bump `ratatui = "0.30"` and the tui-feature MSRV to 1.88; no API rewrites are required by the grep of its usages.** — Every API it uses (init/restore, poll/read, Layout::areas, Flex::Center, Buffer::cell/cell_mut, Marker::Braille, Alignment alias, Block::title(&str), Sparkline/Chart/Gauge/List/Scrollbar, TestBackend + pub Buffer.content) is unchanged in 0.30.2; it does not touch block::Title, Stylize-on-Style, Flex::SpaceAround or exhaustive Marker matches.
  - alternatives: Keep astral-watch on 0.29 and only consume its library API from opsTui (which is the plan anyway — the TUI there stays independent).

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `ratatui` | 0.30.2 | TUI framework facade (core + widgets + crossterm 0.29 backend + macros); init/run/restore helpers; TestBackend | none | verified |
| `ratatui-core` | 0.1.2 | Only if opsTui publishes reusable widget crates; apps use `ratatui` | none | verified |
| `crossterm` | 0.29.0 | Re-exported by ratatui; add directly only for the `event-stream` feature (async EventStream) or osc52 | none | verified |
| `tachyonfx` | 0.25.1 | Shader-like post-render effects (fade, dissolve, sweep, hsl shift, glitch-style compositions), DSL for theme-defined effects | none | verified |
| `tui-big-text` | 0.8.9 | font8x8 big digits/text at Full/Half/Quadrant/Sextant/Octant pixel sizes (clock, readouts) | none (VTE 0.84 draws sextants/octants natively) | verified |
| `tui-bar-graph` | 0.3.5 | Braille/solid bar graph with colorgrad gradients — spectrum analyser bars | none | verified |
| `tui-equalizer` | 0.2.3 | Vertical equalizer bands 0.0–1.0 (audio visualizer alternative) | none | verified |
| `tui-widgets` | 0.7.11 | Umbrella for tui-popup 0.7.6, tui-scrollview 0.6.7, tui-cards, tui-box-text, tui-prompts, tui-qrcode, tui-scrollbar (pick sub-crates individually to limit deps) | none | verified |
| `throbber-widgets-tui` | 0.11.1 | Animated spinners (braille/clock/quadrant sets) for loading states | none | verified |
| `ratatui-image` | 11.0.6 | Image widget (halfblocks/sixel/kitty/iterm2); on this machine halfblocks only | libchafa-dev + libchafa.so if default features are kept (NOT installed); none with default-features=false | verified |
| `ratatui-macros` | 0.7.2 | constraints!/vertical!/horizontal!/line!/span!/text!/row! — already included via ratatui's default `macros` feature | none | verified |
| `palette` | 0.7.7 | HSL/Lab gradients for theme colour ramps via ratatui `palette` feature (Color::from_hsl, From<Srgb>) | none | verified |
| `insta` | 1.48.0 | dev-dependency: snapshot tests of TestBackend output | none (cargo install cargo-insta for review workflow) | verified |
| `color-eyre` | 0.6.5 | Error reports/panic hook (install before ratatui::run so the terminal is restored first) | none | likely |
| `ratatui-termina` | 0.1.0 | Alternative backend added in 0.30.2 — not recommended for v1, listed for awareness | none | verified |

## Risks

- **Full-screen per-cell RGB animation (e.g. a retrowave background gradient plus a 60 fps visualizer over 250x70 = 17,500 cells) may drop well below 60 fps in VTE; upstream saw 24–40 fps at 200x50 all-cells-changing on an M2 Max with 98% of time in write syscalls.** → Keep animated regions small and scoped (tachyonfx with_area), reuse Styles to shrink SGR output, 30 fps default with 60 fps opt-in, dirty-flag rendering for static panels, add a frame-time/changed-cell overlay and benchmark in Ptyxis during arc 1.
- **VTE 0.84 lacks the kitty keyboard protocol (MR !14 unmerged) and DEC 2026 synchronized output; key-release events and some chords are unavailable and tearing can't be prevented by protocol.** → Design bindings around Press events and plain KeyCode/modifiers; gate enhancements on supports_keyboard_enhancement(); accept tearing as a terminal limitation and keep frames small/contiguous.
- **Sixel/Kitty graphics are unavailable in Ptyxis (VTE built without sixel; Ptyxis never enables it) and ratatui-image's default `chafa-dyn` feature fails to build/run without libchafa.** → Use halfblocks (or Canvas octant/braille art) and `ratatui-image` with default-features = false if used at all.
- **tachyonfx Effects are !Send by default; moving them across threads or storing them in shared state will not compile, and the custom 32-bit ms Duration needs conversion from std::time::Duration.** → Keep effects in the render thread's UI state; use EffectTimer::from_ms and convert elapsed time via `.into()`; enable `sendable`/`std-duration` only if genuinely needed.
- **ratatui 0.30.2 declares rust-version 1.88 (higher than the 1.86 announced for 0.30.0) and uses edition 2024; an MSRV CI job pinned lower will fail. `unstable-widget-ref` APIs may change between minor versions.** → Set rust-version = "1.88" and pin the MSRV CI job to it; avoid WidgetRef/FrameExt.
- **Dependencies on ratatui-core vs ratatui version drift (widget crates pin ratatui-core ^0.1.x; a future ratatui 0.31 may bump core) can cause duplicate-crate type mismatches.** → Use `cargo tree -d` in CI, pin exact versions per arc, and update the widget ecosystem together.
- **No Nerd Font is installed: any private-use icon (U+E0B0 powerline, devicons) renders as tofu; braille depends on font fallback (DejaVu Sans / Noto Sans Symbols2).** → Restrict glyphs to box drawing, block elements, braille, sextants/octants (drawn natively by VTE) and ASCII; snapshot tests keep the glyph set explicit; theme files declare their glyph sets.
- **A dedicated input thread blocked in crossterm::event::read() cannot be interrupted at shutdown, and poll/read must never be called from two threads.** → Make the input thread the sole reader, let process exit reap it after ratatui::run restores the terminal, or use poll(timeout) plus a shutdown flag.

## Verified facts

- ratatui 0.30.2 metadata: rust-version 1.88.0; default features [all-widgets, crossterm, layout-cache, macros, underline-color]; optional crossterm_0_28/crossterm_0_29, palette, serde, scrolling-regions, termina/termion/termwiz, unstable-widget-ref (verified with `cargo info ratatui@0.30.2` on this machine)
- ratatui-core 0.1.2, ratatui-widgets 0.3.2, ratatui-crossterm 0.1.2 (default crossterm_0_29), ratatui-termina 0.1.0, ratatui-macros 0.7.2 are the current sub-crates (verified with `cargo info` today)
- Release dates 0.30.0 = 2025-12-26, 0.30.1 = 2026-06-05, 0.30.2 = 2026-06-19 (verified from ratatui CHANGELOG.md on GitHub main)
- 0.30 breaking changes: Marker non_exhaustive, Flex::SpaceAround semantics (use SpaceEvenly), block::Title removed, Style no longer Styled, Alignment→HorizontalAlignment alias, Backend::Error + clear_region, TestBackend Error=Infallible, FromCrossterm/IntoCrossterm, layout-cache feature (verified from BREAKING-CHANGES.md)
- 0.30.1 added Block::shadow(Shadow) with overlay/block/light_shade/medium_shade/dark_shade/symbol/custom presets and CellDiffOption (skip deprecated); 0.30.2 added ratatui-termina and Terminal::apply_buffer/apply_buffer_with_cursor (verified from CHANGELOG and docs.rs Shadow/Terminal pages)
- `ratatui::run<F,R>(f: F) -> R where F: FnOnce(&mut DefaultTerminal) -> R`; try_init enables raw mode + EnterAlternateScreen only, installs a panic hook that restores, no BufWriter; DefaultTerminal = Terminal<CrosstermBackend<Stdout>> (verified from ratatui/src/init.rs on GitHub main)
- Widget trait is `fn render(self, area: Rect, buf: &mut Buffer) where Self: Sized`; implemented for &str, String, Option<W>, Line/&Line, Span/&Span, Text/&Text; docs recommend `impl Widget for &MyWidget`; WidgetRef behind unstable-widget-ref with blanket impl for &W where W: Widget (verified on docs.rs ratatui-core 0.1.2 / ratatui 0.30.2)
- Buffer has pub fields `area: Rect` and `content: Vec<Cell>`; cell/cell_mut take impl Into<Position>; set_string/set_stringn/set_line/set_span/set_style/diff/diff_iter signatures as listed (verified from ratatui-core/src/buffer/buffer.rs on GitHub main)
- Layout::split returns Rc<[Rect]>, areas::<N>, try_areas, spacers, split_with_spacers; kasuari solver; thread-local LRU cache DEFAULT_CACHE_SIZE=500 behind layout-cache (verified on docs.rs ratatui-core Layout)
- Canvas marker resolutions: Braille 2x4, Octant 2x4, Sextant 2x3, Quadrant 2x2, HalfBlock 1x2, Block/Bar/Dot 1x1; shapes Line/Points/Rectangle/Circle/Map/FilledLine/Label (verified on docs.rs ratatui-widgets canvas)
- BorderType has 12 variants incl. six dashed sets and QuadrantInside/Outside; MergeStrategy::{Replace,Exact,Fuzzy} in ratatui_core::symbols::merge (verified on docs.rs)
- crossterm 0.29.0 features: default [bracketed-paste, events, windows, derive-more], optional event-stream, osc52, serde, use-dev-tty, libc; 0.29 added OSC52, query_keyboard_enhancement_flags, is_*/as_* event helpers, rustix 1.0 (verified with cargo info and crossterm CHANGELOG)
- crossterm 0.29 Event enum: FocusGained, FocusLost, Key, Mouse, Paste, Resize(u16,u16) with is_key_press/as_key_press_event/as_mouse_event/as_resize_event helpers; terminal module has BeginSynchronizedUpdate/EndSynchronizedUpdate and supports_keyboard_enhancement (verified on docs.rs crossterm 0.29.0)
- tachyonfx 0.25.1 depends on ratatui-core ^0.1.2; features default [std, dsl], sendable, std-duration, wasm; EffectRenderer<T>::render_effect(&mut self, effect: &mut T, area: Rect, last_tick: Duration) implemented for Frame and Buffer; Duration is a custom {milliseconds: u32} type alias; EffectTimer::from_ms(u32, Interpolation) and From<(u32, Interpolation)>; Effect is !Send; DSL entry EffectDsl::new().compiler().compile(str) (verified with cargo info + docs.rs pages)
- tui-big-text 0.8.9 (MSRV 1.88) PixelSize variants Full, HalfHeight, HalfWidth, Quadrant, ThirdHeight, Sextant, QuarterHeight, Octant; builder pixel_size/style/lines/centered (verified on docs.rs + README)
- tui-widgets 0.7.11 sub-crates and versions: tui-bar-graph 0.3.5 (colorgrad, Braille/Solid, VerticalGradient), tui-equalizer 0.2.3, tui-popup 0.7.6, tui-scrollview 0.6.7, tui-cards 0.3.5, tui-box-text 0.3.4, tui-prompts 0.6.7; throbber-widgets-tui 0.11.1 MSRV 1.88, ratatui ^0.30 (verified with cargo info / docs.rs)
- ratatui-image 11.0.6: rust-version 1.86; default features [image-defaults, crossterm, chafa-dyn]; chafa-dyn links libchafa via pkg-config and needs libchafa.so at runtime; protocols Halfblocks/Sixel(icy_sixel)/Kitty/Iterm2; Picker::from_query_stdio/halfblocks (verified with cargo info + docs.rs + README); libchafa is NOT installed here (ldconfig -p and dpkg -l show none)
- Local terminal stack: libvte-2.91-0 0.84.0-2, libvte-2.91-gtk4-0 0.84.0-2, ptyxis 50.1-1ubuntu2, TERM=xterm-256color, COLORTERM=truecolor, VTE_VERSION=8400 (verified via dpkg -l and env)
- VTE meson_options.txt (master) defines option 'sixel' type boolean value false; Ubuntu vte2.91 debian/rules (ubuntu/devel) passes no -Dsixel; Debian bug #1059446 (enable SIXEL) closed wontfix 2025-07-14; /usr/bin/ptyxis has no undefined symbol vte_terminal_set_enable_sixel (objdump -T) → Sixel unavailable in Ptyxis here
- VTE kitty keyboard protocol: GitLab MR GNOME/vte!14 'kitty keyboard protocol implementation' state=opened, created 2025-12-14, updated 2026-04-11, merged_at null (verified via GitLab API JSON); VTE 0.84.0 tarball dated 2026-03-14, 0.84.1 dated 2026-08-01 (download.gnome.org listing)
- VTE 0.84.0 src/minifont.cc natively draws box drawing U+2500–257F, block elements U+2580–259F, sextants U+1FB00–1FB3B and octants U+1CD00–1CDE5 (octant_value table); braille is not in minifont (verified by fetching the file at tag 0.84.0)
- Font coverage on this machine (fc-list :charset=): U+1FB00 only in Noto Sans Symbols2; U+1CD00 in no installed font; U+2800 braille in DejaVu Sans (non-Mono), DejaVu Serif, Noto Sans Symbols2; U+E0B0 only in OpenSymbol; default monospace = DejaVu Sans Mono
- Upstream performance data: discussion #579 — 200x50 every-cell fg+bg animation ~3 fps initially, 24–40 fps after optimisation on M2 Max, 98% of samples in write; stderr ~11–12 fps vs stdout; issue #1338 — 60 fps static redraw = 7% of a core in release, 50% in debug, diff dominates; maintainer note that diff stops paying off past 30–40% changed cells (verified via GitHub fetch and gh api)
- ratatui FAQ recommends stdout over stderr and a single-threaded get-event→update→render loop unless other parts need async; official templates: hello-world, simple, simple-async (tokio + EventStream), event-driven, event-driven-async, component (verified on ratatui.rs and templates README)
- TestBackend: new(w,h), with_lines, buffer(), resize, assert_buffer, assert_buffer_lines, assert_cursor_position, scrollback; Error=Infallible; Display prints the buffer; ratatui.rs snapshot recipe uses insta::assert_snapshot!(terminal.backend()) and notes colours are not asserted; insta latest 1.48.0 (verified on docs.rs, ratatui.rs, cargo search)
- astral-watch tui.rs (1695 lines, ratatui 0.29, Cargo.toml says tui feature MSRV 1.88) uses ratatui::init/restore, event::poll/read, execute!(EnableMouseCapture), Layout::vertical/horizontal + areas/split, Flex::Center, Buffer::cell(Position)/cell_mut((x,y)), Marker::Braille, Alignment::Center, Block::title(&str), BorderType::Rounded, Sparkline/Chart/Gauge/List/Scrollbar/Clear/Paragraph, TestBackend + buffer().content; it does not import Stylize or use block::Title/Flex::SpaceAround (verified by grep on /home/mattbeam/workspace/astral-watch/src/tui.rs)

## Open questions

- Actual VTE/Ptyxis throughput for the real opsTui layout (250x70, 30–60 fps visualizer region + effects) — needs an in-terminal benchmark (frame time + changed-cell count) in arc 1; upstream numbers are from macOS terminals.
- Whether VTE 0.84 answers XTWINOPS 14/16 t so ratatui-image's from_query_stdio can derive the font size (otherwise halfblocks assume a 4:8 px cell) — not tested because no TTY was available to this subagent.
- Whether tachyonfx::Duration implements From<std::time::Duration> (docs page for the alias didn't show impls); if not, convert with Duration::from_millis(elapsed.as_millis() as u32).
- Whether Ptyxis/Ubuntu will ever enable Sixel (upstream still calls it not ready); if the user wants image protocols, a different terminal (foot/kitty/wezterm/ghostty) is the only path.
- Whether VTE 0.84 supports DEC 2026 synchronized output — evidence (GNOME Terminal DECRPM reply 4) says no, but this was not verified against the VTE 0.84 source/NEWS (NEWS could not be fetched from GitLab).
- Whether to consume astral-watch as a library (current plan) or also reuse its TUI panels; the port cost is near zero either way, but its std-thread design and panic-hook chaining should be replicated in opsTui.

## Sources

- https://raw.githubusercontent.com/ratatui/ratatui/main/BREAKING-CHANGES.md
- https://raw.githubusercontent.com/ratatui/ratatui/main/CHANGELOG.md
- https://ratatui.rs/highlights/v030/
- https://docs.rs/ratatui/0.30.2/ratatui/index.html
- https://raw.githubusercontent.com/ratatui/ratatui/main/ratatui/src/init.rs
- https://raw.githubusercontent.com/ratatui/ratatui/main/ratatui-crossterm/src/lib.rs
- https://raw.githubusercontent.com/ratatui/ratatui/main/ratatui-core/src/buffer/buffer.rs
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/widgets/trait.Widget.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/trait.WidgetRef.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/terminal/struct.Terminal.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/layout/struct.Layout.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/buffer/struct.Cell.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/style/enum.Color.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/symbols/index.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/symbols/merge/enum.MergeStrategy.html
- https://docs.rs/ratatui-core/0.1.2/ratatui_core/backend/struct.TestBackend.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/index.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/block/struct.Block.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/block/struct.Shadow.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/borders/enum.BorderType.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/canvas/index.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/sparkline/struct.Sparkline.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/barchart/struct.BarChart.html
- https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/chart/struct.Chart.html
- https://docs.rs/ratatui-macros/0.7.2/ratatui_macros/index.html
- https://raw.githubusercontent.com/crossterm-rs/crossterm/master/CHANGELOG.md
- https://docs.rs/crossterm/0.29.0/crossterm/event/index.html
- https://docs.rs/crossterm/0.29.0/crossterm/event/enum.Event.html
- https://docs.rs/crossterm/0.29.0/crossterm/terminal/index.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/index.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/fx/index.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/struct.Effect.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/struct.EffectTimer.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/trait.EffectRenderer.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/dsl/index.html
- https://docs.rs/tachyonfx/0.25.1/tachyonfx/type.Duration.html
- https://docs.rs/tui-big-text/0.8.9/tui_big_text/index.html
- https://raw.githubusercontent.com/ratatui/tui-widgets/main/tui-big-text/README.md
- https://docs.rs/tui-bar-graph/latest/tui_bar_graph/index.html
- https://docs.rs/tui-equalizer/latest/tui_equalizer/index.html
- https://docs.rs/throbber-widgets-tui/0.11.1/throbber_widgets_tui/index.html
- https://docs.rs/ratatui-image/11.0.6/ratatui_image/index.html
- https://docs.rs/ratatui-image/11.0.6/ratatui_image/picker/struct.Picker.html
- https://docs.rs/ratatui-image/11.0.6/ratatui_image/protocol/index.html
- https://raw.githubusercontent.com/ratatui/ratatui-image/master/README.md
- https://ratatui.rs/faq/
- https://ratatui.rs/recipes/testing/snapshots/
- https://ratatui.rs/concepts/rendering/under-the-hood/
- https://ratatui.rs/concepts/event-handling/
- https://raw.githubusercontent.com/ratatui/templates/main/README.md
- https://raw.githubusercontent.com/ratatui/templates/main/simple-async/README.md
- https://raw.githubusercontent.com/ratatui/templates/main/event-driven-async/README.md
- https://github.com/ratatui/ratatui/discussions/579
- https://github.com/ratatui/ratatui/issues/1338
- https://gitlab.gnome.org/GNOME/vte/-/raw/master/meson_options.txt
- https://gitlab.gnome.org/GNOME/vte/-/raw/0.84.0/src/minifont.cc
- https://gitlab.gnome.org/api/v4/projects/GNOME%2Fvte/merge_requests/14
- https://gitlab.gnome.org/GNOME/vte/-/issues/2601
- https://github.com/kovidgoyal/kitty/discussions/9293
- https://git.launchpad.net/ubuntu/+source/vte2.91/plain/debian/rules?h=ubuntu/devel
- https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=1059446
- https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=1064823
- https://download.gnome.org/sources/vte/0.84/
- https://docs.otty.sh/vt/terminal-comparison
- local: /home/mattbeam/workspace/astral-watch/Cargo.toml and src/tui.rs; cargo info / cargo search; dpkg -l; objdump -T /usr/bin/ptyxis; strings libvte-2.91-gtk4.so.0; fc-list charset queries; ldconfig -p
