<!-- Judge verdict. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Judge verdict — showcase

SHOWCASE UX, THEMING & LAYOUT — does the design actually deliver a striking, themeable grid dashboard whose components look right from 1x1 to full-screen on THIS terminal (Ptyxis/VTE 0.84, 250x70, DejaVu-class fonts, no Sixel), with a Winamp component and visualizer that feel alive, htop/nvtop/astral-watch parity, and an edit mode. Two claims were checked rather than trusted: (a) ratatui 0.30.2's `set_panic_hook` (init.rs:566) calls `restore()` before the original hook, so any per-frame `catch_unwind` design leaves the terminal restored (alt screen exited, raw mode off) while the app keeps drawing; (b) grid-unit arithmetic on 250x70 with a 1-cell gap: 24 columns gives a 1x1 inner area of 7x8 cells (Tiny, pixel aspect 4.7:10 = tall) and 2x1 of 17x8; 12 columns gives 1x1 inner 17x8 (Small, pixel aspect 9.9:10 = square), 4x2 inner 80x20, 6x3 inner 122x31. The digest's per-component 1x1 plans all assumed ~20x5 units, so a 24-column grid silently invalidates them. The `Rows::Auto` formula copied from the digest into opstui-pragmatic (`H*columns*cell_aspect/W`) evaluates to 3 rows on this terminal (units 10x22); the correct expression `H/((W/columns)*cell_aspect)` gives 13.

**Winner under this lens:** patchbay (contract-first workspace) — under this lens it is the only design whose 1x1..full-screen story survives measurement on the user's terminal (12-column grid gives a square 17x8 Small tile; verified), the only one that declares per-component tiers with minimum sizes rather than relying on class-switch conventions, the only one with a complete theme schema (flourish, effect hooks, per-component overrides, terminal theme) and shell-owned uniform chrome, and it has the most coherent audio->Winamp pipeline. Its weaknesses are delivery (thin arc 1, no roadmap, crate sprawl) and one real bug (catch_unwind vs ratatui's restoring panic hook), all of which the synthesis can fix without touching the layout/theme design.

## Scores (1–10 each; total /60)

| proposal | modularity | performance | extensibility | testability | session velocity | showcase | total |
|---|---|---|---|---|---|---|---|
| opstui (pragmatic core-first, single crate) | 6 | 8 | 6 | 7 | 8 | 6 | **41** |
| patchbay (contract-first workspace) | 9 | 8 | 9 | 8 | 5 | 9 | **48** |
| opstui (store-first, one store many views) | 8 | 8 | 8 | 10 | 6 | 7 | **47** |

### opstui (pragmatic core-first, single crate)

**Strengths**

- Only proposal whose arc 1 puts a screenshot-worthy page on screen: cpu + gpu + pins (the user's own astral-watch parity) in retrowave, with `S` screenshot-to-text and a `--stats` overlay that measures VTE throughput on the real 250x70 layout in the first session.
- Explicitly paints `Role::Bg` over the whole frame before chrome and cells ("paints Bg over the whole frame first"), which is the detail that stops the terminal background leaking through gutters in a themed dashboard; also forces rounded-corner themes to `BorderMode::Each` because `MergeStrategy::Exact` cannot merge rounded glyphs.
- Components are `render(&self)` with per-component `fps()` so only the audio tile drives 60 fps; `set_visible(_, class)` backs sources off when hidden; pins never stop so the alarm banner works on every page.
- Clear per-arc roadmap (v0.1..v0.5) and a 9-step add-a-component checklist inside one crate: highest session velocity of the three.
- `deny.toml` turns the research's do-not-use findings (tokio, cpal, mpris, LGPL ansi_colours) into CI failures instead of review comments.

**Weaknesses**

- Size classes are hand-waved at the small end: with its own 24x6 grid a 1x1 tile has a 7x8 inner area, yet the text promises "Tiny = big CPU% + sparkline" via `widgets/big.rs` (3x3 or Quadrant digits: '37%' needs 11-20 columns). The digest's 1x1 plans (`↓ 12.4 MB/s`, VU pair, watts badge) also do not fit 7 columns. The clock in the example config is placed as 2x1, tacitly admitting 1x1 is unusable.
- `Rows::Auto = clamp(round(H·columns·cell_aspect/W), 3, 12)` yields 3 rows on 250x70 (verified: 3.26), i.e. 10x22-cell units — the formula is inverted.
- Theme is the thinnest of the three: `Theme` has roles/gradients/glyphs/borders/title only — no `flourish` (sun, grid floor, big clock), no effect hooks, no `[components.<kind>]` overrides, no `terminal` (ANSI-16) theme; tachyonfx is deferred to arc 4, so 'retrowave' in arcs 1-3 is a recolour of 'modern' plus gradient titles.
- Edit mode is arc 4 (v0.4) — the product goal names re-arranging the grid explicitly; hot reload is arc 2. Under this lens that is the latest schedule of the three.
- Audio bypasses the `Source`/`Feed` seam ("a reader thread pushes f32 frames into `Arc<Mutex<Ring>>`", "DSP runs in `tick()` on the render thread"), so the visualizer cannot be replayed or snapshot-tested like everything else, and the Winamp mini-spectrum's promised cross-component read (`Feeds::get<T>` "player -> audio bars") has no feed to read — bars only exist inside the audio component's tick.
- No `Shape` hint and no explicit per-component tier list with minimum sizes; a single `min_inner()` plus class-driven `render_<class>` functions means the 'looks great at every footprint' guarantee is by convention, not by declaration.

### patchbay (contract-first workspace)

**Strengths**

- The only proposal whose footprint story survives measurement: 12 columns x 6 rows makes a 1x1 an ~17x8 inner Small tile that is square in pixels (verified 9.9:10), so a tui-big-text Quadrant number plus a sparkline genuinely fits; 4x2 (80x20) and 6x3 (122x31) match the digest's nvtop-header / htop-table assumptions exactly.
- Explicit `tiers()` with minimum inner sizes, richest first, chosen by the shell from the real inner Rect ("nominal footprint is informational"), with `SizeClass` demoted to steering titles and glyph density — this is the htop meter-mode / WidgetKit pattern the research recommended, stated as a contract rather than a convention.
- Complete theme schema: roles, `$palette`, `inherits`, Oklab gradient LUTs, glyph tiers with `chart_marker = "octant_if_vte"`, border sets + merge strategy, `[title]`, `[flourish]` (grid_floor, sun, big_clock, marquee), declarative `[effects]` hooks with `budget_ms` and ambient CRT off by default, `[components.<kind>]` overrides, plus a `terminal` theme that follows the Ptyxis palette. Effects are data in core and mapped to tachyonfx only in the app crate.
- Shell-owned chrome ("the shell draws the frame, components only draw their inner Rect") guarantees uniform borders/titles/health chips across every tile — the single biggest lever for a dashboard reading as one designed surface.
- Coherent 'alive' pipeline: the audio service publishes instance-agnostic 64-band `AudioFrame`s at <=60 Hz; the visualizer and the Winamp component each resample to their bar count and apply their own ballistics (`preset = "winamp"` gravity/peaks vs `"cava"`), `RedrawPolicy::Animated { fps: 60 }` while visible, pw-record killed after 30 s without interest. Winamp tiers (status, shade, main, main+art, full) are the most fully specified, including greyed shuffle/repeat when the player lacks the properties and stream mode when `length = None`.
- Capability probe + `Manifest.requires/optional` + placeholder tiles that print the fix (`usermod -aG i2c`, udev rule) — a degraded dashboard still looks intentional; `patchbay doctor` exposes the same table.
- Theme/config hot reload with notify + debounce is in arc 1, which is what fast visual iteration on retrowave actually needs.

**Weaknesses**

- Per-frame `catch_unwind` around `render` conflicts with ratatui's own panic hook: verified in ratatui 0.30.2 `init.rs:566-571`, the hook calls `restore()` (leave alt screen, disable raw mode) before the original hook, so a caught component panic still trashes the terminal while the app continues drawing. Needs a render-guard-aware hook (thread-local 'in render' flag) or the feature should be dropped.
- Arc 1 shows almost nothing: clock + cpu without the process table, no effects, two themes. For a showcase project the first approved commit is a contract demo, not a dashboard; astral-watch parity (the user's signature) has no scheduled arc at all — there is no roadmap beyond arc 1.
- 18 crates (core, app, cli, testkit, 7 services, 8 components) for a personal dashboard: every new component is a new crate + Cargo feature + CI-matrix line + `register()` plumbing; the per-session cost is the highest of the three.
- No way for a component to opt out of the shell-drawn themed border — the classic Winamp main window (its own title bar, no box border) and htop's full-page F-key bar want custom or borderless chrome; the manifest has no `chrome`/`borderless` flag and `BorderMode::None` is grid-wide only.
- Snapshots use TestBackend text, so theme colour regressions are caught only by hand-written `cell((x,y)).fg` assertions; `RenderCtx.now` is a real `Instant`, so marquee/effect frames are not byte-deterministic under replay.
- `RedrawPolicy::Animated` and `Feed::interest` are elegant but the proposal never states that `Role::Bg` is painted over the full frame each draw — a themed page with gutters will leak the terminal background unless that is added.

### opstui (store-first, one store many views)

**Strengths**

- Best theming safety net: snapshots are `dump::ansi(&buffer)` so colours are part of the golden file ("unlike TestBackend's text-only Display"), and `opstui shot --demo --theme retrowave --size 200x60 --at 45s --out docs/img/overview.svg` regenerates README images headlessly in CI so screenshots never rot — exactly the tooling a showcase repo needs.
- Deterministic rendering: `ViewCx.now: Ts` from a virtual `Clock`, so the Winamp marquee ("220 ms steps from cx.now"), gravity/peak ballistics and alert pulses replay byte-for-byte; a full-app determinism test hashes frames per tick.
- `Store::resample(key, span, buckets, agg)` makes any scalar drawable at any width — the mechanism that lets one metric feed a 1x1 badge, a 2x1 sparkline and a 6x3 chart with identical units and history; `Shape::{Wide,Tall,Squarish}` hint is passed to views.
- Alerts as `AlertId` transitions with a `rules.toml` engine (hold/clear hysteresis, metric-vs-metric RHS like `gpu.temp_c >= gpu.slowdown_c - 5`) plus astral-watch's Lifecycle upstream; the overlay keys on transitions so the banner cannot flicker, and `alert_active` drives the theme's pulse hook.
- Edit mode is arc 2 (earliest of the three) with pure ops and toml_edit save; `sources` and `alerts` debug tiles fall out of the store for free.
- Demand levels `Hidden/Visible/Focused` per source and `fps_hint` per component keep the process under 2 % CPU beside a game while the visualizer still runs at 60.

**Weaknesses**

- Same 24x6 grid as the pragmatic proposal, so the 1x1 tier is a 7x8 inner Tiny tile; its own canonical snapshot rects start at 10x3 (Tiny) and 20x5 (Small), i.e. the tested 'small' sizes are larger than the real 1x1 on this terminal. "1x1 big-number + sparkline" and "1x1 `↓ ↑` rates + link dot" do not fit.
- `fps_hint = 60` only "when focused": an unfocused visualizer runs at 30 fps, which makes the spectrum visibly choppier than the digest's 60-fps-when-visible recommendation — the tile that most needs to feel alive is the one that is usually not focused.
- Effects, flourish and tachyonfx hooks are listed but unscheduled ("Deferred to arcs 2–6: ... tachyonfx hooks"); pins/astral-watch, audio and winamp are all deferred with no arc numbers, so the showcase pieces have no committed order.
- Per-instance UI state as `Box<dyn Any>` downcast on every `on_key`/`view` call, and `Command` values for every side effect (even `Ack`), is ceremony that slows component authoring; each component touches three crates (keys catalogue, source, component).
- Arc 1 is overloaded with infrastructure (rules engine, journal, shot/SVG dumper, sources tile) at the expense of the user's own component — pins and the alert banner are not in the first screenshot.
- Never states that `Role::Bg` is painted over the whole frame first; theme `for_component` overrides exist but no borderless/custom-chrome option for a Winamp skin.

## Best ideas from the non-winners

- store-first: colour-carrying snapshots — snapshot an ANSI/cell dump of the Buffer (fg/bg/modifiers included) instead of TestBackend's text-only Display, so a retrowave-vs-modern regression is a reviewed diff, not a missed cell assertion.
- store-first: `opstui shot --demo --theme X --size WxH --page N --at T` headless SVG/ANSI screenshot command, regenerated by CI so README showcase images never rot; pair it with pragmatic's `S` in-app screenshot key for bug reports.
- store-first: a virtual `Clock`/`Ts` passed in the render context (never `Instant::now()` inside a component) so marquee steps, gravity/peak ballistics, blink and effect timers are byte-deterministic under replay and in snapshots.
- store-first: `Store::resample(key, span, buckets, agg)` (patchbay has a similar `Series::resample`) as THE way sparklines/charts size themselves to any inner width — keep it, and add the `Shape::{Wide,Tall,Squarish}` hint.
- store-first: `rules.toml` generic threshold alerts with hold/clear hysteresis feeding the same `AlertId`-transition overlay as astral-watch's Lifecycle, plus `sources` and `alerts` debug tiles that cost nothing.
- store-first: schedule edit mode in arc 2 (pure ops + HJKL move / Ctrl-hjkl resize / `w` save via toml_edit) — the product goal names rearranging; do not let it slip to v0.4.
- pragmatic: put real, showcase-worthy components in arc 1 — at minimum cpu + gpu + the pins/astral-watch tile with its red alert banner — so the first approved commit is a dashboard screenshot, not a contract demo; and publish an explicit arc -> minor-version roadmap.
- pragmatic: paint `Role::Bg` over the entire frame before chrome and cells every draw; force rounded-corner themes to `BorderMode::Each` because `MergeStrategy::Exact` cannot join rounded glyphs.
- pragmatic: `--stats`/F12 overlay (frame time p50/p95, changed cells, bytes written) in arc 1 to answer the VTE throughput question on the real 250x70 layout, and `deny.toml` bans for tokio/cpal/mpris/LGPL so research verdicts become CI failures.
- pragmatic: selectable DSP smoothing presets (`winamp` falloff/16 + accelerating peak caps, `cava` gravity/integral/monstercat) and 1 Hz mtime-stat hot reload as the zero-dependency fallback if notify proves fussy.

## Concerns for the synthesis

- Grid unit size decides whether 1x1 tiles are usable: on 250x70 with gap 1, 24 columns gives a 7x8-cell inner 1x1 (Tiny, tall) while 12 columns gives 17x8 (Small, square in pixels). The digest's per-component 1x1 plans assumed ~20x5 units. Either adopt 12 columns (per-page override allowed) or re-derive every 1x1/Tiny tier for 7 columns; do not ship 24 columns with the current tier plans. Also fix the `Rows::Auto` formula: the digest/pragmatic expression `H*columns*cell_aspect/W` gives 3 rows here; the correct `H/((W/columns)*cell_aspect)` gives 13.
- patchbay's per-frame `catch_unwind` around `Component::render` is unsound with ratatui 0.30.2's panic hook (init.rs:566-571 calls `restore()` before the original hook). Either install a custom hook that skips `restore()` while a thread-local render guard is set, or drop per-cell panic isolation and rely on the no-panic proptest sweep.
- Effects and flourishes need an arc number and a measured budget, not 'later': the retrowave theme is only 'showcase' if gradient titles, the sun/grid-floor flourish in empty slots, the focus fade and the alert `hsl_shift` pulse exist by arc 2. Keep ambient CRT off by default with the `budget_ms` watchdog, area-scope every effect, and redraw only when `effects.running() || dirty`.
- Give components a chrome option (`Manifest.chrome = Themed | Borderless | Custom`): the Winamp classic main window and htop's full-page F-key bar must be able to draw their own title bar/border, otherwise the shell-owned frame flattens the two most recognisable looks.
- Route audio through the same data seam as everything else (patchbay's 64-band `AudioFrame` feed, or store-first's `audio.bands{L,R}` vector keys); the pragmatic design's render-thread DSP on a raw ring breaks replay, snapshots and the Winamp mini-spectrum. Run the visualizer at 60 fps whenever VISIBLE (not only focused) and kill pw-record after N seconds without interest.
- Snapshot testing must capture colours: TestBackend's Display drops styles, so a theme regression in `nearest_256` or a gradient LUT is invisible. Snapshot a cell dump with fg/bg/modifiers, and run the matrix at the REAL inner sizes the grid produces on 120x40 and 250x70, not just round canonical rects.
- Glyph reality check before committing glyph sets: the digests disagree on the actual Ptyxis font (GNOME `monospace-font-name` says Ubuntu Sans Mono 11 — which lacks rounded corners, eighth blocks, quadrants and braille — while `fc-match` resolves to DejaVu Sans Mono). VTE draws U+2500–259F, sextants and octants itself, but braille depends on font fallback. A 30-second visual test of rounded corners, `▁▂▃▄▅▆▇█`, braille and an octant in Ptyxis should gate the default `chart_marker` and the `unicode` tier.
- Arc 1 sizing: all three proposals overload the first session. Synthesis should cap arc 1 at core + grid + two themes + demo/replay + two real components and put pins (astral-watch parity with the alert banner) no later than arc 2; publish the arc -> version roadmap so the user can plan sessions.
- Paint `Role::Bg` over the whole frame first every draw, load a real `mono` theme under NO_COLOR (crossterm emits no colour SGR at all), and keep a `terminal` theme mapped to ANSI-16 so the dashboard can follow the Ptyxis palette; make `[components.<kind>]` overrides and `inherits` part of the loader from day one.
- Edit mode collisions, undo and `toml_edit` save are specified identically in all three (good); ensure the file watcher ignores self-writes (content hash) and that hot-reloaded themes fire the `theme_swap` effect — otherwise theme iteration, the main showcase workflow, feels broken.
