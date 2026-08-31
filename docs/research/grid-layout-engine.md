<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# opsTui grid layout engine: prior art, model choice, size classes, pages/zoom/focus/edit mode, TOML schema + hot reload, borders/gaps, core Rust types

# Grid layout engine for opsTui — research + design

## 1. Prior art (what each one actually does)

| System | Model | Verified detail |
|---|---|---|
| **Grafana (classic)** | Fixed 24-column grid, `gridPos {x,y,w,h}`; `h` in 30 px row units; page scrolls vertically; "negative gravity" pulls panels up into empty space | docs: `"w": 1-24 (the width of the dashboard is divided into 24 columns)`, `"h": In grid height units, each represents 30 pixels`; example `"gridPos": {"x":0,"y":0,"w":12,"h":9}` |
| **Grafana 12 dynamic dashboards** | Two page modes: *Custom* (the classic grid) and *Auto grid* (min column width Narrow/Standard/Wide/Custom, max columns ≤10, row height Short/Standard/Tall, "fill screen"); rows/tabs nest ≤3 levels; show/hide rules by variable/query/time-range (auto grid only) | grafana.com create-dashboard docs |
| **Android app widgets** | Launcher cell grid; provider declares `targetCellWidth/Height` (API 31), `minResizeWidth/Height`, `maxResizeWidth/Height`; host reports actual dp; app supplies `RemoteViews(Map<SizeF, RemoteViews>)` — i.e. **breakpoint → alternative layout**, chosen by the host from the real size. Cell→dp formula (5×4 handset) portrait `(73n−16)×(118m−16)` | developer.android.com appwidgets/layouts |
| **iOS WidgetKit** | Discrete families only: `systemSmall/Medium/Large/ExtraLarge` (+ accessory families). Widget lists `.supportedFamilies([...])`, view switches on `@Environment(\.widgetFamily)`; **no free resize** | Apple WidgetFamily docs (JS-only page; confirmed via search results) |
| **Home Assistant sections** | Each section is **12 columns**; card returns `getGridOptions() → {rows, columns, min_rows, max_rows, min_columns, max_columns}`; `columns: "full"`; cell ≈30 px wide, 56 px tall, 8 px gap; view has "max number of sections wide" and "dense section placement" (auto-fills gaps, loses order control); "Precise mode" exposes the finer grid in the resize UI | developers.home-assistant.io custom-card; home-assistant.io/dashboards/sections |
| **wtfutil** | tview `Grid`: `wtf.grid.columns: [35,35,35,35]`, `rows: [10,10,10,10,4]` — track sizes **in character cells** (tview: positive = absolute, 0 = default, negative = proportional); modules get `position: {top, left, height, width}` in **track indices/spans**; per-module `refreshInterval`, `enabled`; tview `AddItem(p,row,col,rowSpan,colSpan,minGridHeight,minGridWidth,focus)` shows an item only if the grid is ≥ minGridHeight×minGridWidth — the same primitive can be added several times for responsive layouts | wtfutil.com/configuration; pkg.go.dev rivo/tview#Grid |
| **sampler** | Virtual **80×40** grid (`console.ColumnsCount = 80`, `RowsCount = 40`); `position: [[x, y], [w, h]]` (`Position [][]int`); cell = `Dx()/80`, `(Dy()-statusbar)/40`, rects floored, min dimension enforced; modes `ModeComponentSelect → Enter → menu {Move, Resize, Pinpoint, Resume}`, arrows move selection to nearest component in direction (`moveSelection`), mouse click selects (`findComponentAtPoint`); on exit `if layout.WerePositionsChanged() { config.Update(...) }` re-marshals the whole YAML (so **comments are lost**) | sampler layout.go / config.go / event/handler.go |
| **btop** | Fixed 4 boxes with percent splits (`Cpu 100%×32%`, `Mem 45×40`, `Net 45×28`, `Proc 55×68`), per-box min sizes (`cpu 60×8, mem 36×10, net 36×6, proc 44×16, gpu 41×8`), booleans `cpu_bottom / mem_below_net / proc_left`; **presets** string: `"cpu:1:default,proc:0:default cpu:0:default,mem:0:default,net:0:default cpu:0:block,net:0:tty"` (max 9, preset 0 = all boxes); too-small → blocking screen "Terminal size too small: Width = … Height = …" where digit keys toggle boxes to shrink requirements | btop README + btop.cpp/btop_draw.cpp |
| **tmux** | Binary split tree, layout string `bb62,159x48,0,0{79x48,0,0,79x48,80,0}` (checksum + WxH,x,y + `{}` h-split / `[]` v-split); presets even-horizontal/vertical, main-horizontal/vertical(-mirrored), tiled; `resize-pane -Z` zoom toggles the active pane to the whole window; `select-pane -L/-R/-U/-D`, `swap-pane` | man tmux |
| **zellij** | KDL tree: `pane split_direction="vertical" { pane size="80%"; pane size=5 }`, `focus=true`, `borderless=true`, `stacked=true`, `tab name=…`, `pane_template`/`tab_template` with `children`; **swap layouts** with `max_panes/min_panes/exact_panes` constraints choose an alternative tree by pane count (Alt+[ ]) | zellij.dev creating-a-layout, swap-layouts |
| **i3/sway** | Container tree; `splith/splitv/stacking/tabbed`; `focus left/right/up/down/parent/child`; `move left …`; `fullscreen toggle`; `resize grow width 10 px or 10 ppt` | i3 userguide |
| **ratatui 0.30.2** | 1-D constraint splits solved by **kasuari 0.4.12** (cassowary fork): `Layout::{horizontal,vertical}(constraints).margin().flex(Flex).spacing(impl Into<Spacing>)`, `.split(Rect) -> Rc<[Rect]>`, `.areas::<N>()`, `.try_areas()`, `.spacers()`, `.split_with_spacers()`, `Rect::layout(&Layout)`; thread-local LRU cache (500, `Layout::init_cache`, feature `layout-cache`); `Spacing::{Space(u16), Overlap(u16)}` with `From<i16>` (negative → Overlap); `Block::merge_borders(MergeStrategy::{Replace,Exact,Fuzzy})` merges overlapping borders into ┬┼┤ junctions; constraint priority Min > Max > Length > Percentage > Ratio > Fill (strengths in source: MIN/MAX = STRONG·100, LENGTH = STRONG·10, PERCENTAGE = STRONG, RATIO = STRONG/10, FILL_GROW = MEDIUM, GROW = MEDIUM/10, spacers WEAK; FLOAT_PRECISION_MULTIPLIER = 100) | docs.rs 0.30.2 + local ratatui-0.29.0 source (same solver code) |

Takeaways: every "dashboard" product (Grafana, HA, wtfutil, sampler) converged on **fixed-count unit grids with x/y/w/h placements**; every *terminal multiplexer* uses **split trees**; every *widget platform* uses **host-reported real size → alternative layouts** (Android breakpoints, iOS families, HA min/max grid options).

## 2. Design choice: fixed unit grid (recommended), with ratatui Layout inside cells

Trade-offs:
- **Fixed unit grid (Grafana/wtfutil/sampler):** free re-arrangement is trivial (move/resize = integer arithmetic on `Placement`), footprints are first-class, config is human-readable, hit-testing and edit-mode ghosts are one division. Cost: holes are possible and stretched units on odd terminal shapes — acceptable for a dashboard.
- **Constraint/weight rows+columns (btop, cassowary):** great for 1-D, awkward for 2-D with spanning; cassowary over-constrained results are "approximate" and order-dependent; not invertible for mouse editing.
- **Nested splits (tmux/zellij/i3):** no holes, natural resize, but "move component across the tree" is tree surgery, footprints aren't expressible, and a dashboard-like equalized look is hard. Zellij's swap-layouts are worth borrowing as *pages/presets*, not as the base model.
- **CSS Grid via taffy 0.14** (grid feature default): full auto-placement/spans in f32 — overkill; rounding to cells and hit-testing you'd still write yourself.

**Recommendation: hybrid.** Page = fixed grid of `columns` (default **24**) × `rows` (default **6**, per-page override) scaled to the body `Rect`; placements in units; the engine maps units → `Rect` with a pure integer function (no solver, no thread-local cache, deterministic, unit-testable); ratatui `Layout`/`Constraint`/`Flex` is used *inside* components and for chrome (tab bar / body / status bar via `Layout::vertical([Length(1), Fill(1), Length(1)])`).

**Track algorithm** (per axis; `len` cells, `n` tracks, `gap` cells between tracks):
```
usable = len - gap*(n-1)
start(i) = floor(i*usable/n) + i*gap
end(i)   = floor((i+1)*usable/n) + i*gap      // exclusive
span (x, w) -> [start(x), end(x+w-1))
```
Widths differ by ≤1 cell, sums are exact, monotonic → invertible for mouse (`cell_at(pos)` by binary search over `start`). In *shared-border* mode use `gap = 0` and extend every non-last span by +1 cell so neighbours overlap one column/row (exactly what `Spacing::Overlap(1)` does in ratatui; you can even validate the integer function against `Layout::horizontal(vec![Constraint::Fill(1); 24]).spacing(Spacing::Overlap(1)).split(area)` in tests).

**Cell aspect.** Terminal cells are ≈1:2 (DejaVu Sans Mono is the effective font here: advance ≈0.60 em, line height ≈1.17 em → w/h ≈ 0.52; Ptyxis `font-name 'Monospace 10'`, `fc-match monospace → DejaVu Sans Mono`). For a footprint `w×h` (units) to look square in pixels you need `w·unit_w·0.52 ≈ h·unit_h`, i.e. with square-in-cells units the 2:1 family (2×1, 4×2, 6×3) is square-ish and 1×1 is a tall tile. With 24 columns on a 200-col terminal a unit is ~8 cells wide, so choose rows ≈ `H / 8` (6–7 on a 55-line terminal). Provide `grid.rows = "auto"`: `rows = clamp(round(H·columns / (W·cell_aspect⁻¹)), 3, 12)` with `cell_aspect = 0.5` in config — and let the config author fix `rows` per page when they want stretch-to-fill behaviour instead. Never make rendering depend on the nominal footprint (see §3).

**Too small — degradation ladder** (evaluated each frame, cheapest first):
1. *Configured*: gaps, own borders.
2. *Dense*: `gap 0` + shared borders + compact titles (saves ~1 cell per track per axis).
3. *Starved cells*: if a cell's inner `Rect` < `Component::min_size(SizeClass::Tiny)` the component is drawn as a placeholder chip (`▪ cpu`) — components must be zero-size safe (ratatui widgets are; our code must not index buffers).
4. *Below `grid.min_terminal`* (default 80×24): switch page to **stack mode** — placements sorted by `(priority desc, y, x)` laid out with `Layout::vertical(Constraint::Min(min_h))` and a scroll offset (own offset or `tui-scrollview 0.6.7`), or in the MVP a btop-style notice "Terminal too small: need 80×24 for page Overview (now 60×18)". Dropping lowest-priority placements (Grafana "hide" rules, btop digit toggles) is offered as an explicit key in that notice, not silently.

## 3. Size classes from the real Rect

Compute from the **inner** area (after borders/padding), never from the nominal footprint (a 6×3 on a 100-col terminal is smaller than a 4×2 on a 300-col one):

```
width : XS <12  S <24  M <48  L <96  XL
height: XS <3   S <6   M <12  L <24  XL
SizeClass = min(width_class, height_class)   // Tiny, Small, Medium, Large, Huge
```
Also hand the component the raw `Rect` and a `Shape` hint (`Wide` if w > 2·h·cell_aspect⁻¹…, `Tall`, `Squarish`) so e.g. the htop component picks "CPU bars in columns" vs "rows". Components declare `supported_footprints()` (iOS-style list used by the picker and the `s` cycle-size key), `min_size(class)` and `preferred_footprint()`; resizing is free within `[min_footprint, max_footprint]` (Android-style) but the picker snaps to supported sizes.

Trait (data updates are separate from rendering; rendering is `&self` so a zoomed or duplicated view never disturbs history):
```rust
pub struct RenderCtx<'a> { pub area: Rect /*inner*/, pub class: SizeClass, pub shape: Shape,
    pub focused: bool, pub zoomed: bool, pub dense: bool, pub theme: &'a Theme, pub now: Instant }
pub trait Component: Send {
    fn kind(&self) -> &'static str;
    fn title(&self, class: SizeClass) -> Cow<'_, str>;       // short titles for Tiny/Small
    fn supported_footprints(&self) -> &'static [Footprint];  // e.g. [1x1, 2x1, 4x2, 6x3]
    fn preferred_footprint(&self) -> Footprint;
    fn min_size(&self, class: SizeClass) -> Size;             // inner cells
    fn priority(&self) -> Priority;                           // for stack/drop order
    fn tick(&mut self, now: Instant);                         // pull from sampler threads/channels
    fn render(&self, ctx: &RenderCtx, buf: &mut Buffer);     // or frame: &mut Frame
    fn handle_key(&mut self, key: KeyEvent) -> Handled { Handled::No }
    fn handle_mouse(&mut self, m: MouseEvent, local: Position) -> Handled { Handled::No }
    fn keymap(&self) -> &'static [KeyHint] { &[] }            // shown in the status bar
}
```
This mirrors ratatui's component template (`init/handle_key_event/handle_mouse_event/update/draw`) but drops its tokio `Action` channel (astral-watch is std-threads; sampler threads + `std::sync::mpsc` are enough for v0).

## 4. Pages, zoom, focus, mouse, edit mode

- **Pages**: `Vec<Page>`, hotkeys `1–9` (btop presets / tmux windows), `[`/`]` prev/next, tab bar via `Tabs` hidden when there is one page. Each page has its own focus and its own `rows` override.
- **Zoom** (`z`, tmux `-Z`): `zoomed: Option<InstanceId>`; the body rect goes entirely to that component (class recomputed → usually `Huge`). Esc/`z` restores.
- **Focus**: `Tab`/`Shift-Tab` in reading order `(y, x)`; `hjkl`/arrows spatial: candidates whose orthogonal projection overlaps the focused rect and which lie strictly in the direction; pick min edge distance, tie-break by centre offset; fallback nearest centre in the half-plane (sampler's `moveSelection`, i3 `focus left`). Focused cell gets `theme.border_focused` (wtfutil's `border.focused`).
- **Mouse** (crossterm 0.29 `EnableMouseCapture`, `MouseEvent { kind, column, row, modifiers }`, `MouseEventKind::{Down(MouseButton), Up, Drag, Moved, ScrollDown, ScrollUp, …}`): keep `Vec<(InstanceId, Rect)>` from the last solve; click → focus (`Rect::contains(Position)`), double-click → zoom, wheel → forward to the component with `local = pos - inner.as_position()`; in edit mode `Drag` moves (grid delta = `cell_at(drag) - cell_at(press)`), dragging the bottom-right corner glyph resizes.
- **Edit mode** (`e`): state machine `Normal → Edit{Select | Move | Resize | Picker | Confirm}`. Keys: `HJKL` move one unit, `Ctrl-hjkl` resize (shrink with Shift), `s`+dir swap with the neighbour (only when the target has the same footprint — otherwise move with collision check), `a` picker (fuzzy list of registered kinds and existing instances → default footprint dropped into the first free spot: first-fit scan in reading order, like HA dense placement), `x`/`Del` remove, `u` undo / `Ctrl-r` redo (undo stack = `Vec<Page>` snapshots; cheap), `w`/`Ctrl-s` save, `Esc` leave (prompt if dirty), `d` dense toggle, `?` help. Collision policy: reject with red ghost (Grafana pushes; sampler allows overlap — neither is what you want on a fixed grid). Edit mode renders a dotted background grid and unit coordinates in the status bar.
- **Save back preserving comments**: `toml_edit 0.25.13` (`str::parse::<DocumentMut>()`, `doc["pages"]` → `ArrayOfTables`, mutate `at`/`size` values in place, `doc.to_string()`); comments/whitespace survive except on removed items and dotted-key reordering. Write atomically (temp file + rename in the same dir), then re-parse with `toml` and compare to the in-memory model before declaring success; remember the content hash so the watcher ignores your own write. Sampler's "marshal the whole struct" approach loses comments — don't do that.

## 5. Config schema (TOML)

```toml
schema = 1
[grid]
columns = 24
rows = 6                # or "auto"
gap = 1                 # cells; ignored when borders = "shared"
borders = "each"        # "each" | "shared" | "none"
cell_aspect = 0.5       # terminal cell w/h, only for the "auto" rows heuristic
min_terminal = { cols = 80, rows = 24 }
[theme]
name = "retrowave"

[[components]]          # instances (a kind may appear several times)
id = "net-lan"
kind = "network"
refresh_ms = 1000
[components.options]
interface = "eno1"

[[components]]
id = "net-wifi"
kind = "network"
options = { interface = "wlp7s0" }

[[components]]
id = "pins"
kind = "astral"          # astral_watch::i2c::read_reading on a thread
options = { bus = "auto", warn_amps = 8.5 }

[[pages]]
name = "Overview"
hotkey = "1"
rows = 6
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
```
Rust side: `#[derive(Deserialize)] #[serde(deny_unknown_fields)]` structs with `#[serde(default)]`, `options: toml::Table` handed to the kind's factory (`fn(&toml::Table) -> Result<Box<dyn Component>>`) which deserialises its own typed `Options`. Semantic validation after parse: unique ids/hotkeys, kind registered, `at+size` inside `columns×rows`, pairwise overlap check reporting both ids, footprint outside `supported` → warning. Use `toml::de::Error::span()` (toml 1.1.4) to print `file:line:col`. Defaults come from `impl Default` and `opstui config default` prints them; `opstui config check` validates; the CSS-`grid-template-areas`-style `areas = """cccc gggg"""` string can be added later as sugar that lowers to placements. **figment 0.10.19 is not recommended**: it pins `toml ^0.8` (duplicate toml in the tree) and the layering you need (defaults ← file ← `OPSTUI_*` env ← CLI) is ~30 lines by hand.

**Hot reload**: watch the *parent directory* (`~/.config/opstui/`) — editors save via rename/truncate, so filter events by path; use `notify 8.2.0` + `notify-debouncer-full 0.7.0` (the 9.0.0-rc.5 / 0.8.0-rc.2 pair on crates.io today is a release candidate), `new_debouncer(Duration::from_millis(250), None, handler)`, `debouncer.watch(dir, RecursiveMode::NonRecursive)`. On event: parse → validate → diff: keep component instances whose `(kind, options)` are unchanged (history survives), rebuild the rest, swap `Arc<Config>`; on error keep the old config and show a toast — never exit.

## 6. Gaps, borders, titles, dense mode

- `borders = "each"`: `Block::bordered().border_type(theme.border)` per cell, `gap ≥ 1` (modern theme: Rounded + gap 1; retrowave: Double/Thick + magenta/cyan).
- `borders = "shared"` (dense): `gap 0`, spans extended by one cell so neighbours overlap, `Block::bordered().merge_borders(MergeStrategy::Exact)` (0.30 recipe: `Layout::horizontal([Fill(1); 2]).spacing(Spacing::Overlap(1))` + `merge_borders(MergeStrategy::Exact)`). `Exact` only merges when a composite glyph exists (`│`+`━` → `┿`), so shared mode should use Plain/Thick/Double, not Rounded (use `Fuzzy` if you insist on rounded corners; it downgrades to plain at junctions). Draw cells in reading order so later borders merge into earlier ones.
- `borders = "none"`: 1-line inline header (title left, badges right via `Line::right_aligned`) and background tints per theme, gap 1.
- Titles: `Block::title_top(Line)` left; status badges (alert, paused, interface name) top-right; `Tiny` class → glyph only; `Small` → `title(SizeClass::Small)` short form; focus shown by border colour and a `▸` marker; `dense` also hides the tab bar when there is one page.

## 7. Core types sketch

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Footprint { pub w: u8, pub h: u8 }

#[derive(Clone, Serialize, Deserialize)]
pub struct GridSpec { pub columns: u8, pub rows: Rows, pub gap: u8, pub borders: BorderMode,
                      pub cell_aspect: f32, pub min_terminal: Size }
pub enum Rows { Fixed(u8), Auto }
pub enum BorderMode { Each, Shared, None }

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement { pub id: InstanceId, pub at: [u8; 2], pub size: [u8; 2],
                       #[serde(default = "default_priority")] pub priority: u8 }

pub struct Page { pub name: String, pub hotkey: Option<char>, pub rows: Option<u8>,
                  pub place: Vec<Placement> }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeClass { Tiny, Small, Medium, Large, Huge }
impl SizeClass {
    pub fn of(inner: Size) -> Self {
        let w = match inner.width  { 0..=11 => 0, 12..=23 => 1, 24..=47 => 2, 48..=95 => 3, _ => 4 };
        let h = match inner.height { 0..=2  => 0, 3..=5   => 1, 6..=11  => 2, 12..=23 => 3, _ => 4 };
        [Self::Tiny, Self::Small, Self::Medium, Self::Large, Self::Huge][w.min(h)]
    }
}

pub struct Cell { pub id: InstanceId, pub placement: Placement, pub outer: Rect, pub inner: Rect,
                  pub class: SizeClass, pub starved: bool }
pub struct Solved { pub mode: SolveMode /* Grid | Dense | Stack | TooSmall */,
                    pub cells: Vec<Cell>, pub col_starts: Vec<u16>, pub row_starts: Vec<u16> }

pub struct LayoutEngine;
impl LayoutEngine {
    pub fn tracks(len: u16, n: u8, gap: u16) -> Vec<(u16, u16)> {
        let n16 = u32::from(n); let usable = u32::from(len.saturating_sub(gap * (u16::from(n) - 1)));
        (0..n16).map(|i| { let s = (i * usable / n16) as u16 + gap * i as u16;
                           let e = ((i + 1) * usable / n16) as u16 + gap * i as u16; (s, e) }).collect()
    }
    pub fn solve(&self, spec: &GridSpec, page: &Page, body: Rect,
                 min_of: &dyn Fn(&InstanceId, SizeClass) -> Size) -> Solved {
        let rows = spec.rows_for(body);
        let gap = if spec.borders == BorderMode::Shared { 0 } else { spec.gap as u16 };
        let cols = Self::tracks(body.width, spec.columns, gap);
        let rws  = Self::tracks(body.height, rows, gap);
        let overlap = u16::from(spec.borders == BorderMode::Shared);
        let cells = page.place.iter().map(|p| {
            let (x0, _) = cols[p.at[0] as usize]; let (_, mut x1) = cols[(p.at[0] + p.size[0] - 1) as usize];
            let (y0, _) = rws[p.at[1] as usize];  let (_, mut y1) = rws[(p.at[1] + p.size[1] - 1) as usize];
            if p.at[0] + p.size[0] < spec.columns { x1 += overlap; }
            if p.at[1] + p.size[1] < rows         { y1 += overlap; }
            let outer = Rect::new(body.x + x0, body.y + y0, x1 - x0, y1 - y0).intersection(body);
            let inner = spec.borders.inner(outer);           // Block::inner() equivalent
            let class = SizeClass::of(inner.as_size());
            let starved = inner.as_size() < min_of(&p.id, SizeClass::Tiny);
            Cell { id: p.id.clone(), placement: p.clone(), outer, inner, class, starved }
        }).collect();
        Solved { mode: SolveMode::Grid, cells, col_starts: cols.iter().map(|c| c.0).collect(),
                 row_starts: rws.iter().map(|r| r.0).collect() }
    }
    pub fn hit<'a>(&self, s: &'a Solved, pos: Position) -> Option<&'a Cell> {
        s.cells.iter().find(|c| c.outer.contains(pos))
    }
    pub fn cell_at(&self, s: &Solved, body: Rect, pos: Position) -> Option<(u8, u8)> { /* binary search on *_starts */ None }
}
```
Edit-mode ops are pure functions on `Page` (`move_by`, `resize_by`, `swap`, `insert_first_fit`, `remove`) returning `Result<Page, EditError::{Overlap(id), OutOfBounds, BelowMin}>`, which makes them trivially unit- and property-testable (e.g. proptest: no two placements overlap after any op sequence; `tracks()` sums to `len`).

## Integration notes
- astral-watch's existing TUI (`src/tui.rs`, ratatui 0.29, `run_tui(bus, addr, interval, cfg, auto, metrics, shutdown)`) already does a poor-man's size class (`compact = area.height < 16`) and nests `Layout::vertical/horizontal` with `Percentage` splits — the opsTui `astral` component should call the library (`astral_watch::i2c::read_reading`) on its own thread and re-implement the pin bars per `SizeClass`, not embed `run_tui`.
- Put the engine + `Component` trait in a workspace crate (`opstui-core`) that depends on `ratatui-core 0.1.2` (ratatui's guidance for widget crates), and the binary on `ratatui 0.30.2` with `crossterm_0_29`.


## Recommendations

- **Use a fixed unit grid per page (24 columns × N rows, N default 6, per-page override, optional `rows = "auto"` from terminal aspect) with x/y/w/h placements, mapped to Rects by a pure integer track function; use ratatui Layout/Constraint/Flex only inside components and for chrome.** — Every dashboard product (Grafana gridPos, Home Assistant 12-col sections, wtfutil tview Grid, sampler 80×40) converged on this; it makes move/resize/swap integer arithmetic, is invertible for mouse hit-testing and edit ghosts, is deterministic and unit-testable, and footprints are first-class. Cassowary is 1-D and approximate under conflict; split trees make free re-arrangement tree surgery.
  - alternatives: Nested split tree (tmux/zellij/i3 style, KDL) — better for multiplexer-like use; taffy 0.14 CSS Grid — full auto-placement but f32 and overkill; ratatui `Layout::horizontal(vec![Fill(1); 24]).spacing(Spacing::Overlap(1))` for tracks — equivalent and usable as a test oracle.
- **Derive SizeClass (Tiny/Small/Medium/Large/Huge) from the actual inner Rect with width buckets <12/<24/<48/<96 and height buckets <3/<6/<12/<24, class = min(width_class, height_class); also pass the raw Rect and a Wide/Tall/Squarish shape hint. Components declare supported_footprints(), preferred_footprint(), min_size(class), priority().** — Nominal footprint is meaningless across terminal sizes; Android's SizeF-breakpoint mapping and HA's min/max grid options show the real-size-driven pattern works. Free resize within min/max plus a supported-sizes list gives both iOS-style curated looks and Android-style flexibility.
  - alternatives: iOS-only discrete families (no free resize); wtfutil/tview approach of registering the same component several times with minGridWidth/Height.
- **Degradation ladder when space is short: configured → dense (gap 0 + shared borders + short titles) → starved cells render a placeholder chip → below grid.min_terminal switch to a priority-ordered stack mode with scrolling (MVP: btop-style 'terminal too small' notice with keys to hide components).** — btop blocks with a notice and digit toggles; Grafana/HA hide by rules; a fixed grid cannot reflow, so dropping components does not free space for others — a stack fallback is the only layout that stays useful at 60×18.
  - alternatives: Silently drop lowest-priority placements; scroll the whole grid (tui-scrollview 0.6.7).
- **Keep the TOML config as the source of truth; edit mode saves through toml_edit 0.25.13 (DocumentMut, in-place mutation of [[pages.place]] values) with atomic temp+rename, re-parse verification, and a self-write hash so the watcher ignores it.** — toml_edit preserves comments/formatting on round trip (verified in crate docs); sampler's re-marshal approach loses comments. The user's astral-watch conventions already use TOML.
  - alternatives: RON 0.12.2 (no comment-preserving editor); KDL 6.7.1 (zellij-style, comment-preserving parser exists but unfamiliar); YAML.
- **Hot reload with notify 8.2.0 + notify-debouncer-full 0.7.0 watching the config's parent directory non-recursively, 250 ms debounce, validate-then-swap Arc<Config>, keep old config and toast on error, and diff instances by (kind, options) to preserve component history.** — Editors save via rename/truncate so the file path itself is unreliable; notify docs recommend watching the parent. The 9.0.0-rc.5 / 0.8.0-rc.2 versions on crates.io today are release candidates.
  - alternatives: Poll mtime every second (simplest, no extra crates); notify 9.0.0-rc.5 if you accept an rc.
- **Do not use figment 0.10.19; do defaults ← file ← OPSTUI_* env ← CLI layering by hand with serde defaults, deny_unknown_fields, and toml 1.1.4 spans for file:line:col errors.** — figment pins toml ^0.8, giving two toml crates in the tree, and the layering you need is ~30 lines.
  - alternatives: figment (nice provenance errors), config crate.
- **Borders: three modes — each (own Block, gap ≥ 1), shared (gap 0, spans overlap 1 cell, Block::merge_borders(MergeStrategy::Exact), plain/thick/double border types only), none (inline 1-line header). Title top-left via title_top, badges top-right, Tiny → glyph only, focus by border colour.** — ratatui 0.30's Spacing::Overlap + MergeStrategy::Exact is the supported way to collapse borders; Exact only merges glyphs that exist as composites, so rounded corners need Fuzzy or a non-shared mode.
  - alternatives: Manual border drawing on the Buffer (what pre-0.29 code had to do).
- **Edit mode as a state machine over pure Page operations (move_by/resize_by/swap/insert_first_fit/remove → Result<Page, EditError>) with a snapshot undo stack, collision = reject with red ghost, vim-style keys (HJKL move, Ctrl-hjkl resize, s swap, a picker, x remove, u undo, w save, z zoom, d dense, 1-9 pages, Tab/hjkl focus).** — Pure functions make property tests (no overlaps ever) trivial; snapshotting a Page is cheap; rejecting collisions is the predictable option for a fixed grid (Grafana's gravity push is surprising in a TUI).
  - alternatives: Grafana-style gravity/push; sampler-style menu (Enter → Move/Resize/Resume).
- **Split the workspace: opstui-core (engine, Component trait, SizeClass, config types) depends on ratatui-core 0.1.2; the binary depends on ratatui 0.30.2 with crossterm 0.29.** — ratatui's own guidance is that widget libraries depend on ratatui-core for API stability; it keeps component crates light and testable.
  - alternatives: Single crate.

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `ratatui` | 0.30.2 | Layout/Constraint/Flex/Spacing::Overlap, Rect helpers (layout, centered, contains, columns/rows), Block::merge_borders(MergeStrategy), Tabs, Frame; kasuari solver behind Layout; layout-cache feature | none | verified |
| `ratatui-core` | 0.1.2 | Dependency for the opstui-core crate that defines the Component trait and engine (widget-library guidance); Rect/Buffer/Layout types | none | verified |
| `crossterm` | 0.29.0 | Key/mouse/resize events, EnableMouseCapture, MouseEvent{kind,column,row,modifiers}; enable ratatui feature crossterm_0_29 | none | verified |
| `kasuari` | 0.4.12 | Cassowary solver used internally by ratatui 0.30 (only a direct dep if you ever add custom constraints; not recommended) | none | verified |
| `toml` | 1.1.4+spec-1.1.0 | Typed config parsing via serde; de::Error::span() and Spanned<T> for file:line:col validation errors; toml::Table for per-kind options | none | verified |
| `toml_edit` | 0.25.13+spec-1.1.0 | Comment/format-preserving write-back of edit-mode changes (DocumentMut, ArrayOfTables, Item, Decor); serde feature adds de::from_document / ser::to_document | none | verified |
| `serde` | 1 | Derive for config structs (#[serde(default)], deny_unknown_fields) | none | verified |
| `notify` | 8.2.0 | Filesystem watcher for hot reload (watch parent dir, NonRecursive); latest on crates.io is 9.0.0-rc.5 (release candidate) | none (inotify) | verified |
| `notify-debouncer-full` | 0.7.0 | new_debouncer(timeout, tick, handler) to coalesce editor save bursts and rename tracking; latest is 0.8.0-rc.2 (rc) | none | verified |
| `tui-scrollview` | 0.6.7 | Optional: scrollable stack fallback for tiny terminals (ScrollView::new(Size), render_widget, ScrollViewState) | none | verified |
| `unicode-width` | 0.2.2 | Title/badge truncation to cell widths | none | verified |
| `color-eyre` | 0.6.5 | Config validation error reports with sections/suggestions | none | verified |
| `clap` | 4.6.6 | `opstui --config`, `opstui config check/default`, `--page`, `--dense` | none | verified |
| `figment` | 0.10.19 | Layered config (NOT recommended: pins toml ^0.8 → duplicate toml crate) | none | verified |
| `taffy` | 0.14.0 | CSS Grid/Flexbox solver (grid feature default, f32 units) — alternative engine, not recommended for a cell grid | none | verified |
| `ratatui-interact` | 0.5.3 | FocusManager/ClickRegionRegistry + form widgets on ratatui ^0.30 — evaluated, not needed (spatial focus needs rect-aware nav) | none | verified |
| `ratatui-auto-grid` | 0.1.0 | auto_grid(area, n, spacing) -> Vec<Rect> square-root packing — too small to depend on; write first-fit yourself | none | verified |
| `ron` | 0.12.2 | Alternative config format — not recommended (no comment-preserving editor) | none | verified |
| `kdl` | 6.7.1 | Alternative zellij-style layout format with format-preserving parser — not recommended over TOML for this user | none | verified |

## Risks

- **notify 9 / notify-debouncer-full 0.8 are release candidates; pinning them risks API churn mid-project.** → Pin notify 8.2.0 + notify-debouncer-full 0.7.0 (verified stable on crates.io); or poll mtime every second in v0.
- **Cell aspect ratio is font-dependent; a 6x3 footprint looks square only for ~1:2 cells (DejaVu Sans Mono ≈0.52). Nerd/other fonts or VTE line-spacing change it.** → Expose grid.cell_aspect and rows = "auto"; never let rendering depend on nominal footprint — always SizeClass from the real Rect.
- **MergeStrategy::Exact only merges where composite glyphs exist; Rounded corners and dashed styles produce inconsistent junctions in shared-border mode, and merging depends on draw order.** → Restrict shared mode to Plain/Thick/Double border types per theme, draw cells in reading order, use Fuzzy as the fallback for rounded themes; add a snapshot test (Buffer diff) for a 2x2 shared layout.
- **Comment-preserving save can still lose formatting on removed placements and dotted-key reordering (toml_edit documented limitation), and a save racing with the file watcher can reload stale content.** → Atomic temp+rename, re-parse and compare after write, keep a content hash to ignore self-writes, and back up config.toml → config.toml.bak on first edit-mode save.
- **Tiny cells: components that index the Buffer directly (as astral-watch's tui.rs set_cell does) can panic on zero-width/height areas.** → Engine marks starved cells and draws a placeholder; add a proptest that renders every component at every Rect from 0x0 to 10x4 without panicking.
- **Overlapping or out-of-bounds placements in hand-written TOML.** → Semantic validation pass reporting both ids with toml spans; `opstui config check` subcommand; hot reload rejects invalid files and keeps the old config.
- **ratatui's Layout cache is thread-local LRU (500 entries); rendering from multiple threads or many distinct Rects defeats it — irrelevant if the grid uses the integer track function, but component-internal Layouts still hit it.** → Render on one thread; call Layout::init_cache if many pages/components create distinct layouts.
- **Mouse capture changes terminal selection behaviour (users lose native text selection) and Ptyxis/VTE reports Up/Drag button inconsistently per crossterm docs.** → Make mouse opt-in (`--mouse` / config), Shift-drag remains native selection in VTE; treat Up/Drag as left button.

## Verified facts

- ratatui 0.30.2 Layout API (new/vertical/horizontal, direction, constraints, margin, horizontal_margin, vertical_margin, flex, spacing<T: Into<Spacing>>, split -> Rc<[Rect]>, areas::<N>, try_areas, spacers, split_with_spacers, DEFAULT_CACHE_SIZE=500, init_cache) — docs.rs/ratatui/0.30.2 fetched today
- ratatui 0.30.2 uses the kasuari solver (docs.rs Layout page says so; ratatui-core 0.1.2 features list kasuari) and Spacing::{Space(u16), Overlap(u16)} with From<u16>/From<i16>/From<i32> — docs.rs fetched
- ratatui 0.30.2 Block has merge_borders(MergeStrategy) with variants Replace/Exact/Fuzzy (docs.rs symbols/merge/enum.MergeStrategy.html fetched; examples: Exact merges │+━ → ┿, Fuzzy merges ┌+┐ → ┬); ratatui.rs collapse-borders recipe uses Layout::horizontal([Fill(1);2]).spacing(Spacing::Overlap(1)) + Block::bordered().merge_borders(MergeStrategy::Exact)
- Constraint priority Min > Max > Length > Percentage > Ratio > Fill (docs.rs Constraint page); solver strengths from the locally cached ratatui-0.29.0 source: MIN_SIZE_GE/MAX_SIZE_LE = STRONG*100, LENGTH_SIZE_EQ = STRONG*10, PERCENTAGE_SIZE_EQ = STRONG, RATIO_SIZE_EQ = STRONG/10, MIN/MAX_SIZE_EQ = MEDIUM*10, FILL_GROW = MEDIUM, GROW = MEDIUM/10, SPACE_GROW = WEAK*10, ALL_SEGMENT_GROW = WEAK, FLOAT_PRECISION_MULTIPLIER = 100; results cached in a thread-local LruCache keyed on (Layout, Rect)
- Rect 0.30.2 methods: new, area, is_empty, left/right/top/bottom, inner(Margin), outer(Margin), offset, resize, union, intersection, intersects, contains(Position), clamp, rows/columns/positions, as_position, as_size, centered/centered_horizontally/centered_vertically, layout::<N>/layout_vec/try_layout — docs.rs fetched
- Flex variants Legacy(default)/Start/End/Center/SpaceBetween/SpaceEvenly/SpaceAround — docs.rs fetched; 0.30 highlights: SpaceEvenly replaces old SpaceAround behaviour, Alignment→HorizontalAlignment, Rect::layout(), ratatui::run(), WidgetRef blanket reversed (impl Widget for &W), MSRV 1.86/edition 2024
- ratatui 0.30.2 features via `cargo info`: default = all-widgets, crossterm, layout-cache, macros, underline-color; crossterm_0_28 / crossterm_0_29 feature flags; rust-version 1.88
- crates.io versions via cargo search/info today: toml_edit 0.25.13+spec-1.1.0 (rust 1.85, features parse/display/serde), kasuari 0.4.12, notify 9.0.0-rc.5 latest with 8.2.0 stable, notify-debouncer-full 0.8.0-rc.2 latest with 0.7.0 stable, notify-debouncer-mini 0.7.0, ron 0.12.2, taffy 0.14.0 (grid/flexbox/block features default), tui-scrollview 0.6.7, ratatui-interact 0.5.3 (ratatui ^0.30), ratatui-auto-grid 0.1.0, tui-dashboard 0.1.2 (WIP), kdl 6.7.1, figment 0.10.19
- figment 0.10.19 normal deps (crates.io API): atomic ^0.6, serde ^1, uncased ^0.9.3, optional toml ^0.8, serde_json ^1, serde_yaml ^0.9, pear ^0.2, parking_lot ^0.12, tempfile ^3 — i.e. it pins toml 0.8 while the project would use toml 1.1.4
- toml 1.1.4 provides Spanned<T>::span() -> Range<usize> and de::Error::span()/message(); does not depend on toml_edit (uses toml_parser/toml_writer) — docs.rs fetched
- toml_edit crate docs: parse with str::parse::<DocumentMut>(), doc["key"] indexing, comments preserved on round trip, order of dotted keys not preserved, formatting of removed items lost; serde helpers de::{from_str, from_slice, from_document}, ser::{to_string, to_string_pretty, to_vec, to_document} behind the serde feature — docs.rs fetched
- notify docs: recommended_watcher, Watcher::watch(path, RecursiveMode)/unwatch, editors truncate or replace files so precise events differ per editor (watch the parent dir); notify-debouncer-full: new_debouncer(Duration, Option<Duration>, handler), debouncer.watch(path, RecursiveMode), rename tracking via FileIdMap — docs.rs fetched
- Grafana JSON model docs: gridPos w 1-24 (24 columns), h in 30 px units, x/y same units, example {x:0,y:0,w:12,h:9}, 'negative gravity that moves panels up'; Grafana 12 dynamic dashboards: Custom vs Auto grid (min column width Narrow/Standard/Wide/Custom, max columns ≤10, row height Short/Standard/Tall/Custom, fill screen), rows/tabs nest to 3 levels, show/hide rules only in Auto grid — grafana.com fetched
- Home Assistant developer docs: getGridOptions() returns rows, columns, min_rows, max_rows, min_columns, max_columns; 'Each section is divided in 12 columns'; columns:'full'; cells ≈30px wide, 56px tall, 8px gap; sections view has 'Max number of sections wide' and 'Dense section placement' — fetched
- Android appwidget docs: targetCellWidth/targetCellHeight (API 31), minResizeWidth/Height, maxResizeWidth/Height, RemoteViews(Map<SizeF, RemoteViews>) responsive mapping, OPTION_APPWIDGET_SIZES, portrait formula (73n−16)×(118m−16) dp — developer.android.com fetched
- wtfutil sample config (wtfutil.com/configuration): wtf.grid.columns: [35,35,35,35], rows: [10,10,10,10,4], module position {top,left,height,width}, refreshInterval, enabled; tview Grid semantics: SetColumns/SetRows positive=absolute, 0=default, negative=proportional; AddItem(p,row,col,rowSpan,colSpan,minGridHeight,minGridWidth,focus) shows an item only if the grid is at least that size — pkg.go.dev fetched
- sampler source: console.ColumnsCount = 80, RowsCount = 40; ComponentConfig.Position [][]int ([[x,y],[w,h]]); layout maps units to cells with floor(Location*columnWidth) and enforces a minimum dimension; modes Default/Intro/Pause/ComponentSelect/MenuOptionSelect/ComponentMove/ComponentResize/ChartPinpoint; arrows select nearest component, Enter opens menu {Move, Resize, Pinpoint, Resume}, mouse click selects; handler saves via config.Update when WerePositionsChanged() (full yaml.Marshal rewrite) — raw GitHub files fetched
- btop: presets format 'box_name:P:G' (P = alternate position 0/1, G = graph symbol), max 9 presets, preset 0 = all boxes, default presets string quoted; shown_boxes; box min sizes cpu 60×8, mem 36×10, net 36×6, proc 44×16, gpu 41×8; percent splits Cpu 100/32, Mem 45/40, Net 45/28, Proc 55/68; too-small loop prints 'Terminal size too small: Width = … Height = …' and digit keys toggle boxes — README + btop.cpp/btop_draw.cpp fetched
- zellij layouts (zellij.dev): pane split_direction="vertical", size=5 / size="80%", focus=true, name, borderless=true, command, plugin location, tab name=…, tab_template/default_tab_template/pane_template with children, stacked=true, cwd; swap_tiled_layout/swap_floating_layout with max_panes/min_panes/exact_panes and Alt+[ ] switching
- tmux man page: resize-pane -Z toggles zoom to the whole window; layout string 'bb62,159x48,0,0{79x48,0,0,79x48,80,0}'; presets even-horizontal/even-vertical/main-horizontal(-mirrored)/main-vertical(-mirrored)/tiled; i3 userguide: container tree with splith/splitv/stacking/tabbed, focus left/right/up/down/parent/child, move, fullscreen toggle, resize grow/shrink px|ppt
- ratatui component template Component trait: register_action_handler, register_config_handler, init(Size), handle_events, handle_key_event, handle_mouse_event, update(Action), draw(&mut Frame, Rect) — raw GitHub components.rs fetched
- crossterm 0.29 MouseEvent { kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers }, MouseEventKind::{Down(MouseButton), Up, Drag, Moved, ScrollDown, ScrollUp, ScrollLeft, ScrollRight}, EnableMouseCapture/DisableMouseCapture — docs.rs fetched
- Local: astral-watch Cargo.toml pins ratatui 0.29 optional (feature tui), toml 0.9, std threads; src/tui.rs uses Layout::vertical/horizontal with Percentage/Length/Min and a `compact = area.height < 16` size switch; lib.rs exposes `pub mod tui` only behind the tui feature; run_tui(bus, addr, interval, cfg, auto, metrics, shutdown)
- Local: Ptyxis font-name 'Monospace 10' with use-system-font true; desktop monospace 'Ubuntu Sans Mono 11' is not installed so fc-match monospace resolves to DejaVu Sans Mono (Book) — cell aspect ≈1:2 follows from that font's metrics (advance ≈0.60 em, line ≈1.17 em; metrics from memory, not measured)
- Local: cargo registry already caches ratatui-0.29.0, crossterm-0.28.1, cassowary-0.3.0 (from astral-watch); no ratatui 0.30 or toml_edit cached yet

## Open questions

- Rows per page: fixed default (6) vs `auto` derived from terminal aspect — which should be the shipped default? (Affects whether a page designed on the 4K workstation stretches or gains rows on a laptop.)
- Should shared-border ('dense') mode be a theme property (retrowave = shared double lines) or a runtime toggle independent of theme? Proposed: both — theme sets the default, `d` overrides per session.
- Collision policy in edit mode: reject (proposed) vs Grafana-style push-down vs swap-on-drop — confirm preference before implementing the pure Page ops.
- Config file location/name (~/.config/opstui/config.toml vs XDG + per-page files) and whether pages should be separable files (`pages/*.toml`) for sharing.
- Whether the astral component should link astral-watch as a library dependency (i2c group needed at runtime, MSRV 1.85) or talk to the exporter on 127.0.0.1:9942 — affects only the component, not the engine, but influences the options schema (`bus`, `addr` vs `url`).
- Mouse support default on/off (it disables native VTE text selection unless Shift is held).
- Exact size-class breakpoints should be tuned against the real components (htop-like needs ~60 cols for a process table; nvtop-like needs ≥ 12 rows for graphs) — revisit after the first two components exist.
- Is a CSS `grid-template-areas`-style `areas = """…"""` string worth supporting as config sugar in a later arc (harder to round-trip through toml_edit)?

## Sources

- https://docs.rs/ratatui/0.30.2/ratatui/layout/struct.Layout.html
- https://docs.rs/ratatui/0.30.2/ratatui/layout/index.html
- https://docs.rs/ratatui/0.30.2/ratatui/layout/enum.Spacing.html
- https://docs.rs/ratatui/0.30.2/ratatui/layout/struct.Rect.html
- https://docs.rs/ratatui/0.30.2/ratatui/layout/enum.Constraint.html
- https://docs.rs/ratatui/0.30.2/ratatui/layout/enum.Flex.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Block.html
- https://docs.rs/ratatui/0.30.2/ratatui/symbols/merge/enum.MergeStrategy.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/trait.Widget.html
- https://docs.rs/ratatui/0.30.2/ratatui/struct.Frame.html
- https://ratatui.rs/recipes/layout/collapse-borders/
- https://ratatui.rs/highlights/v030/
- https://ratatui.rs/concepts/layout/
- https://github.com/ratatui/templates/blob/main/component/template/src/components.rs
- https://docs.rs/kasuari/0.4.12/kasuari/
- ~/.cargo/registry/src/*/ratatui-0.29.0/src/layout/layout.rs (local solver strengths, cache)
- https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/view-dashboard-json-model/
- https://grafana.com/docs/grafana/latest/visualizations/dashboards/build-dashboards/create-dashboard/
- https://developer.android.com/develop/ui/views/appwidgets/layouts
- https://developer.apple.com/documentation/widgetkit/widgetfamily
- https://developers.home-assistant.io/docs/frontend/custom-ui/custom-card/
- https://www.home-assistant.io/dashboards/sections/
- https://wtfutil.com/configuration/
- https://pkg.go.dev/github.com/rivo/tview#Grid
- https://raw.githubusercontent.com/sqshq/sampler/master/component/layout/layout.go
- https://raw.githubusercontent.com/sqshq/sampler/master/config/config.go
- https://raw.githubusercontent.com/sqshq/sampler/master/config/component.go
- https://raw.githubusercontent.com/sqshq/sampler/master/console/console.go
- https://raw.githubusercontent.com/sqshq/sampler/master/event/handler.go
- https://github.com/aristocratos/btop
- https://raw.githubusercontent.com/aristocratos/btop/main/src/btop.cpp
- https://raw.githubusercontent.com/aristocratos/btop/main/src/btop_draw.cpp
- https://zellij.dev/documentation/creating-a-layout.html
- https://zellij.dev/documentation/swap-layouts.html
- https://man7.org/linux/man-pages/man1/tmux.1.html
- https://i3wm.org/docs/userguide.html
- https://docs.rs/toml_edit/latest/toml_edit/
- https://docs.rs/toml_edit/latest/toml_edit/de/index.html
- https://docs.rs/toml_edit/latest/toml_edit/ser/index.html
- https://docs.rs/toml/latest/toml/
- https://docs.rs/notify/9.0.0-rc.5/notify/
- https://docs.rs/notify-debouncer-full/latest/notify_debouncer_full/
- https://docs.rs/figment/0.10.19/figment/
- https://crates.io/api/v1/crates/figment/0.10.19/dependencies
- https://docs.rs/tui-scrollview/latest/tui_scrollview/
- https://docs.rs/taffy/0.14.0/taffy/
- https://docs.rs/ratatui-interact/latest/ratatui_interact/
- https://github.com/yozhgoor/ratatui-auto-grid
- https://docs.rs/crossterm/0.29.0/crossterm/event/struct.MouseEvent.html
- /home/mattbeam/workspace/astral-watch/Cargo.toml
- /home/mattbeam/workspace/astral-watch/src/tui.rs
- /home/mattbeam/workspace/astral-watch/src/lib.rs
