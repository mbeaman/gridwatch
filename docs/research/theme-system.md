<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# opsTui theme system: first-class "retrowave" / "modern" / phosphor / etc. looks in a ratatui 0.30 truecolor TUI — schema, palettes, file format, colour handling, retro flourishes, hot reload, Rust sketch

## 0. What this machine actually renders (verified today)

- Ptyxis 50.1 on libvte-2.91 **0.84.0-2**; `TERM=xterm-256color`, `COLORTERM=truecolor`, `NO_COLOR` unset; terminfo has `colors#0x100` and **no `Tc`/`RGB` capability** — so capability detection must key off `COLORTERM`, not terminfo. GNOME `color-scheme = prefer-dark`.
- **The terminal font is not DejaVu.** Ptyxis has `use-system-font=true` and GNOME `monospace-font-name = 'Ubuntu Sans Mono 11'` (`fc-match monospace` returns DejaVu Sans Mono, but Ptyxis follows the GNOME setting). Ubuntu Sans Mono **lacks** rounded corners U+256D–2570, eighth blocks U+2594/2595, quadrants U+2596–259F, braille U+2800–28FF, sextants U+1FB00+, ● U+25CF, ▶ U+25B6, ◢ U+25E2 and powerline U+E0B0. DejaVu Sans Mono covers all of U+2500–259F, 25CF/25B6/25E2, but also **not braille** (fontconfig falls back to proportional DejaVu Sans) and not sextants (falls back to Noto Sans Symbols2). Nothing installed covers octants U+1CD00 or powerline glyphs (only OpenSymbol, proportional).
- **VTE draws these itself** (`src/minifont.cc` on master): U+2500–257F box drawing, U+2580–259F blocks/quadrants/shades, U+2571–2573 diagonals, U+23B8–23BD, U+25E2–25E5 triangles, U+1FB00–1FBBF (sextants, eighths, fills), U+1CD00–1CDE5 octants, U+1CE90–1CEAF sixteenths. Consequence: **box drawing, rounded corners, eighth/quadrant/half blocks, shades, sextants and octants are font-independent on Ptyxis**, so `tui-big-text` `PixelSize::Sextant`/`Octant` and `symbols::pixel::{SEXTANTS, OCTANTS}` are usable here (I could not fetch the 0.84 NEWS to pin the exact release each range landed; mark octants "likely"). **Braille is not synthesised** — it renders via fallback to DejaVu Sans, which works in practice but is the one "unicode" glyph family worth a visual smoke test. Powerline/nerd glyphs: no font — the nerd set must stay off by default (as required).

## 1. Theme schema design

**Principle:** components never touch `Color::` or glyph literals; they ask the theme for a *role*, a *gradient id*, a *glyph name*, a *border spec* or an *effect hook*. That is what keeps "modern" clean and "retrowave" loud from one code path.

### 1a. Semantic colour roles (18 + gradients)
`bg`, `surface` (raised area behind widgets), `panel` (component interior), `border`, `border_focused`, `title`, `text`, `text_muted`, `text_ghost` (decorative only, may fail contrast), `accent.primary/secondary/tertiary`, `severity.ok/warn/crit/info`, `selection.fg/bg`, `cursor`, plus `gradients.*` (`load`, `temp`, `power`, `mem`, `net_rx`, `net_tx`, `audio`, `title`) each a list of ≥2 stops low→high. Each role resolves to a ratatui `Color` after (a) `$palette` indirection, (b) capability downsampling, (c) contrast validation.

Values accepted: `"#rrggbb"`, `"#rgb"`, `"$name"` (from `[palette]`), `"indexed:213"`, `"named:light-cyan"` (ratatui `Color::from_str` already parses names, `#RRGGBB` and bare `u8` indices — verified in `ratatui-core/src/style/color.rs`), and `"reset"` (terminal default — btop's empty `main_bg` idiom for transparent themes).

### 1b. Glyph sets
Three tiers, chosen per theme and overridable by CLI: `ascii` (`#=-|+ .:` , `[####  ]`, `^`/`v`/`!`), `unicode` (default; only U+2500–259F + U+25xx shapes, all safe on VTE and DejaVu), `nerd` (`glyphs.nerd = true` opt-in only; never default). Named slots: `bar` (`nine_levels` → ratatui `symbols::bar::NINE_LEVELS` `▏▎▍▌▋▊▉█`, `three_levels`, `ascii`), `sparkline` (same but vertical `symbols::bar` … `▁▂▃▄▅▆▇█`), `chart_marker` (`braille` | `half_block` | `block` | `dot` | `bar` → `symbols::Marker`, which is `#[non_exhaustive]` in 0.30 so match with a wildcard), `gauge_unicode` (→ `Gauge::use_unicode(true)`, 8 sub-cell steps), `shade_ramp` (`symbols::shade::{EMPTY,LIGHT,MEDIUM,DARK,FULL}` = ` ░▒▓█`, the mono/NO_COLOR fallback for gradients), `severity.{ok,warn,crit,info}` glyphs (`● ▲ ■ ◆` unicode / `OK ! !! i` ascii), `arrows.{up,down,rx,tx}`, `pause`, `bullet`.

### 1c. Border sets
`plain | rounded | double | thick | light_double_dashed … heavy_quadruple_dashed | quadrant_inside | quadrant_outside | one_eighth_tall | proportional_wide | custom` — all map 1:1 onto ratatui 0.30 `BorderType` variants (Plain, Rounded, Double, Thick, the six dashed variants, QuadrantInside `▗▄▖▐▌▝▀▘`, QuadrantOutside `▛▀▜▌▐▙▄▟`) or `symbols::border::Set { top_left, top_right, bottom_left, bottom_right, vertical_left, vertical_right, horizontal_top, horizontal_bottom }` for `custom` (e.g. retro corners `◢ ◣ ◤ ◥` — VTE-drawn, or `╔═╗` with `─` sides for a chrome-bezel look). `border_focused_set` may differ (e.g. thick when focused). Since the layout is a grid, use `Block::merge_borders(MergeStrategy::Exact|Fuzzy)` (new in 0.30) so adjacent panels share one line — a theme option `borders.merge = "fuzzy"`.

### 1d. Titles, bars, charts, dim/bold
`title.style = plain | badge | gradient | bracketed`, `title.case = upper|as_is`, `title.position = top|bottom`, `title.alignment`, `title.bold`. `Block::title_top/title_bottom` take `Into<Line>` in 0.30 (the old `block::Title` type is gone), so a gradient title is just a `Line` of per-char `Span`s. Bars: `bars.style = blocks | shade | ascii`, `bars.gradient = <gradient id>`, `bars.show_value`. Sparklines: `Sparkline::data()` accepts `Option<u64>`/`SparklineBar` (per-bar styles — that is how you gradient-colour a sparkline by height) and `direction(RenderDirection::RightToLeft)`. Charts: `Dataset::graph_type(GraphType::{Line,Scatter,Bar,Area})` + `marker`; `Area` (new) with `fill_to_y` gives the synthwave "filled mountain" look for free. Modifiers available in ratatui 0.30: `BOLD, DIM, ITALIC, UNDERLINED, SLOW_BLINK, RAPID_BLINK, REVERSED, HIDDEN, CROSSED_OUT`; theme flags `text.bold_titles`, `text.dim_muted` (DIM on light backgrounds usually *lightens* text → keep it off in light themes), `text.italic_hints`. Never use BLINK (VTE honours it and it is unreadable; also breaks the "modern" promise).

### 1e. Effect hooks (tachyonfx 0.25.1, depends on `ratatui-core ^0.1.2` → compatible with ratatui 0.30.2)
Hooks are named events the app raises; the theme decides what (if anything) plays: `startup`, `theme_swap`, `focus_change`, `alert_enter`, `alert_active`, `alert_clear`, `layout_change`, `ambient`. Concrete mappings (all signatures verified on docs.rs):
- startup sweep: `fx::sweep_in(Motion::LeftToRight, 12, 0, theme.color(Role::Bg), (600, Interpolation::QuadOut))` (signature `sweep_in<T: Into<EffectTimer>, C: Into<Color>>(direction, gradient_length: u16, randomness: u16, faded_color, timer)`).
- theme swap crossfade: `fx::fade_from(old_fg, old_bg, 150)` (`fade_from<T,C>(fg, bg, timer)`).
- alert pulse ("neon glow"): `fx::repeating(fx::ping_pong(fx::hsl_shift(Some([0.0, 0.0, 25.0]), None, (450, Interpolation::SineInOut))))` restricted with `.with_filter(CellFilter::FgColor(crit))` and `.with_area(panel)` (`hsl_shift(hsl_fg_change: Option<[f32;3]>, hsl_bg_change: Option<[f32;3]>, timer)`; panics if both None — check the S/L units in the doc before tuning).
- CRT scanlines/flicker: `fx::effect_fn(state, timer, |st, ctx, cells| …)` darkening every other row's bg by `crt.scanline_darken`; flicker = tiny random lightness jitter per frame. `CellFilter` has `All, Area, RefArea, FgColor, BgColor, Inner(Margin), Outer(Margin), Text, NonEmpty, AllOf, AnyOf, NoneOf, Not, Layout, PositionFn, EvalCell, Static`.
- Rendering: `EffectRenderer::render_effect(&mut self, effect: &mut Effect, area: Rect, last_tick: Duration)` is implemented for `Frame` and `Buffer`; or use `EffectManager<K>` (`add_effect`, `add_unique_effect(key, fx)`, `cancel_unique_effect`, `is_running`, `process_effects(duration, buf, area)`). `tachyonfx::Duration` is a custom `{ milliseconds: u32 }` (feature `std-duration` swaps to std); `EffectTimer: From<u32>`, `From<(u32, Interpolation)>`, `From<Duration>`. 34 easing variants incl. `SineInOut`, `QuadOut`, `Spring`, `SmoothStep`.
- **Performance budget:** effects cost O(cells in area) per frame, per effect. A 200×60 terminal is 12k cells; a full-screen effect_fn at 60 Hz is a few hundred µs in release — fine — but the real cost is *diff bytes*: CRT scanlines change bg on half the screen, so the crossterm diff each frame becomes ~6k cells × ~20 bytes ≈ 120 KB/frame → at 30 Hz that is 3.6 MB/s to the pty and VTE repaints everything. Budget: theme `effects.budget_ms = 4`; measure `Instant` around `process_effects`; if the moving average exceeds budget, auto-disable ambient effects (log once). Only redraw when `manager.is_running() || dirty` (the pattern tachyonfx docs recommend). Ambient effects (scanlines, flicker) are **off even in retrowave** by default and toggled with a key; event effects (sweep/fade/pulse) are bounded to ≤600 ms and to the affected panel area. `--no-effects` / `effects.enabled=false` short-circuits everything.

## 2. Concrete palettes (WCAG contrast computed locally, xterm-256 nearest index computed locally)

**retrowave (original, informed by Synthwave '84 #262335/#ff7edb/#03edf9/#fede5d/#ff8b39/#fe4450 and the requested #ff2975):**
| role | hex | vs bg | 256 |
|---|---|---|---|
| bg | #0b0324 deep indigo | — | 233 |
| surface | #1a0b3d | 1.10 | 234 |
| panel | #241b2f (Synthwave sidebar) | — | — |
| border | #7a3fb5 (≈3.0:1 hand-calc; #5b2a86 was 2.01 → fails 3:1 graphics) | ~3.0 | 97 |
| border_focused / cursor / accent.primary | **#ff2975** hot pink | 5.53 | 198 |
| accent.secondary | #00f0ff cyan | 14.2 | 51 |
| accent.tertiary | #b967ff electric purple | 6.12 | 135 |
| text | #efe9ff | 16.9 | 255 |
| text_muted | #8a7fb0 | 5.46 | 103 |
| ok / warn / crit | #05ffa1 / #fede5d / #fe4450 | 15.0 / 15.0 / 5.85 | 49 / 221 / 203 |
| selection | fg #ffffff on bg #3d1a63 | | |
| gradient `load` (sunset) | #00f0ff → #b967ff → #ff2975 → #ff8b39 | | |
| gradient `title` (chrome) | #f6f0ff → #c8b8ff → #ff2975 → #7a3fb5 | | |
A load bar at 0–100 %: `▏▎▍▌▋▊▉█` cells coloured by `load.at(i/width)` → cyan fading through purple to pink, tip orange when >90 %.

**modern = Catppuccin Mocha** (catppuccin.com/palette): bg Base #1e1e2e (256#235), surface Mantle #181825, panel Surface0 #313244, border Surface1 #45475a, border_focused Lavender #b4befe (9.17), title/text Text #cdd6f4 (11.3), text_muted Subtext0 #a6adc8 (7.37 — Overlay1 #7f849c is 4.44, just under AA, so use it only as `text_ghost`), accent Blue #89b4fa / Mauve #cba6f7 / Teal #94e2d5, ok Green #a6e3a1, warn Yellow #f9e2af, crit Red #f38ba8 (7.08), selection Surface2 #585b70, cursor Rosewater #f5e0dc, gradient `load` Green → Yellow → Peach #fab387 → Red, gradient `title` = none (plain bold Text). No flourishes.

**phosphor-green (P1)** bg #0a0f0a, text #33ff66 (14.4), bright #a8ffb0, muted #1f9a3a (5.29), ghost #0f3d0f, all accents = text; severity by intensity + glyph + REVERSED, gradient = `#0f3d0f → #1f9a3a → #33ff66 → #a8ffb0` (luminance-monotonic, reads in greyscale). **phosphor-amber (P3)** bg #100c00, text #ffb000 (10.7), bright #ffd866, muted #a06d00 (4.36 → bump to #b07a00 for AA), ghost #3d2a00. Both use `bars.style = shade` (` ░▒▓█`) and `border = double` for the bezel. Hex values are conventional CRT approximations, not a standard.

**gruvbox-dark** (gruvbox.vim, each hex paired with its own 256 index): bg #282828 (235), surface dark1 #3c3836 (237), panel dark0_soft #32302f (236), border dark2 #504945 (239), border_focused bright_orange #fe8019 (208), text light1 #ebdbb2 (223), muted gray #928374 (245, 4.02 — large text only), accents #fabd2f/#83a598/#8ec07c, ok #b8bb26 (142), warn #fabd2f (214), crit #fb4934 (167, 4.29 → always BOLD/REVERSED for text), selection dark3 #665c54, gradient #b8bb26 → #fabd2f → #fe8019 → #fb4934. Light variant: bg light0 #fbf1c7 (229), text dark1 #3c3836 (10.2), faded_* accents (#9d0006 crit 7.6, #b57614 warn 3.33 → shape channel required, #79740e ok 4.29).

**dracula** (draculatheme.com): bg #282a36 (236), surface/selection #44475a, border comment #6272a4 (3.03 — graphics OK, text only large), border_focused pink #ff79c6, text #f8f8f2 (13.4), muted #6272a4, accents purple #bd93f9 / pink #ff79c6 / cyan #8be9fd, ok #50fa7b, warn #f1fa8c, crit #ff5555 (4.53), gradient #50fa7b → #f1fa8c → #ffb86c → #ff5555.

**high-contrast-dark** (Okabe-Ito accents, jfly.uni-koeln.de/color): bg #000000, text #ffffff (21), muted #c0c0c0 (11.5), border #ffffff, border_focused #f0e442, accents sky #56b4e9 / orange #e69f00 / purple #cc79a7, ok bluish-green #009e73 (6.14), warn orange #e69f00 (9.32), crit vermilion #d55e00 (5.43), gradient #56b4e9 → #f0e442 → #e69f00 → #d55e00, borders `double`, severity glyphs mandatory. Light twin: invert bg/text, keep Okabe-Ito.

**Also shippable with verified hexes:** nord (bg #2e3440, text #eceff4, frost #88c0d0, ok #a3be8c, warn #ebcb8b, crit #bf616a — note nord3 #4c566a is 1.69:1, unusable as muted text; use nord4 #d8dee9 + DIM), tokyo-night (bg #1a1b26, text #c0caf5, muted #565f89 2.76 → ghost only, blue #7aa2f7, ok #9ece6a, warn #e0af68, crit #f7768e), rosé-pine (bg #191724, surface #1f1d2e, overlay #26233a, text #e0def4, muted #6e6a86, subtle #908caa, love #eb6f92 crit, gold #f6c177 warn, pine #31748f, foam #9ccfd8, iris #c4a7e7, selection #403d52 = highlightMed, cursor #524f67 = highlightHigh). **rosé-pine-dawn** as the light reference: bg #faf4ed, surface #fffaf3, overlay #f2e9e1, text #464261 (per palette.json today; older docs say #575279), subtle #797593 (4.02), muted #9893a5 (2.73 — ghost only), pine #286983 (5.59), love #b4637a (3.84), **gold #ea9d34 is 2.05:1** — the canonical light-theme lesson: warn colour is fine for bars/glyphs, never for body text unless auto-darkened.

## 3. Prior art → opsTui TOML

- **btop `.theme`** (`theme[key]="#hex"`, verified from `themes/dracula.theme`): `main_bg, main_fg, title, hi_fg, selected_bg, selected_fg, inactive_fg, graph_text, meter_bg, proc_misc, cpu_box, mem_box, net_box, proc_box, div_line` + gradient triplets `temp/cpu/free/cached/available/used/download/upload/process_{start,mid,end}`. Lessons: per-widget box colours, three-stop gradients, empty `main_bg` = transparent.
- **bottom** `[styles]`: `theme`, `[styles.widgets] border_colour, selected_border_colour, widget_title, bg_colour, text, selected_text, disabled_text, thread_text`, `[styles.graphs] graph_colour, legend_text`, `[styles.cpu] cpu_core_colours`, `[styles.memory]`, `[styles.network] rx_colour, tx_colour`, `[styles.battery] high/medium/low_battery_colour`; values are `"#hex"`, `"r, g, b"`, ratatui names, or `{ colour, bg_colour, bold }`. Lesson: string-or-table style values; `colour`/`color` both accepted.
- **zellij** KDL: per-UI-component blocks (`text_unselected, text_selected, ribbon_*, table_*, list_*, frame_selected, frame_highlight, exit_code_success/error`) each with `base, background, emphasis_0..3` RGB triplets. Lesson: component-scoped style bundles → our `[components.<id>]` overrides.
- **alacritty** (`[colors.primary] background/foreground/dim_foreground`, `[colors.cursor] text/cursor`, `[colors.selection] text/background`, `[colors.normal|bright|dim] black…white`, `search`, `hints`, `footer_bar`) and **wezterm** (`[colors] foreground background cursor_bg cursor_fg cursor_border selection_fg selection_bg scrollbar_thumb split ansi brights indexed compose_cursor visual_bell`, `[colors.tab_bar.*]`, `[metadata]`). Lesson: 16-ANSI schemes are everywhere → ship an importer.
- **base16/base24** YAML (`system, name, author, variant, palette.base00..base0F`; base24 adds base10/base11 darker backgrounds and base12–17 bright red/yellow/green/cyan/blue/magenta, with an explicit ANSI mapping). Import mapping: base00→bg, base01→surface, base02→selection.bg, base03→text_ghost, base04→text_muted, base05→text, base07→title, base08→crit, base0A→warn, base0B→ok, base0C→tertiary, base0D→primary, base0E→secondary, base10/11→panel/bg when present.

**Chosen format: TOML, one file per theme** in `~/.config/opstui/themes/<name>.toml` (+ embedded built-ins via `include_str!`), with `inherits` and `$palette` indirection. Full example:

```toml
[meta]
name = "retrowave"
schema = 1
variant = "dark"            # dark | light
inherits = "base-dark"      # optional; child keys override parent
author = "mbeaman"

[palette]
indigo = "#0b0324"
indigo2 = "#1a0b3d"
plum = "#241b2f"
violet = "#7a3fb5"
pink = "#ff2975"
cyan = "#00f0ff"
purple = "#b967ff"
orange = "#ff8b39"
mint = "#05ffa1"
sun = "#fede5d"
red = "#fe4450"
snow = "#efe9ff"
dusk = "#8a7fb0"

[colors]
bg = "$indigo"
surface = "$indigo2"
panel = "$plum"
border = "$violet"
border_focused = "$pink"
title = "$snow"
text = "$snow"
text_muted = "$dusk"
text_ghost = "#3d2a63"
cursor = "$pink"
[colors.accent]
primary = "$pink"
secondary = "$cyan"
tertiary = "$purple"
[colors.severity]
ok = "$mint"
warn = "$sun"
crit = "$red"
info = "$cyan"
[colors.selection]
fg = "#ffffff"
bg = "#3d1a63"

[gradients]                 # low -> high, 2..8 stops, interpolated in Oklab
load   = ["$cyan", "$purple", "$pink", "$orange"]
temp   = ["$cyan", "$purple", "$pink", "$red"]
power  = ["$purple", "$pink", "$orange", "$sun"]
mem    = ["$cyan", "$purple"]
net_rx = ["$cyan", "$mint"]
net_tx = ["$purple", "$pink"]
audio  = ["$cyan", "$pink", "$sun"]
title  = ["#f6f0ff", "#c8b8ff", "$pink", "$violet"]

[fallback16]                # optional explicit 16-colour mapping (else nearest)
bg = "black"
text = "white"
border_focused = "light-magenta"
"accent.secondary" = "light-cyan"

[glyphs]
set = "unicode"             # ascii | unicode | nerd
nerd = false                # must be explicitly true to use nerd glyphs
bar = "nine_levels"
sparkline = "nine_levels"
chart_marker = "braille"
gauge_unicode = true
shade_ramp = " ░▒▓█"
[glyphs.severity]
ok = "●"
warn = "▲"
crit = "■"
info = "◆"

[borders]
set = "custom"
focused_set = "thick"
merge = "fuzzy"             # replace | exact | fuzzy
[borders.custom]            # ratatui symbols::border::Set fields
top_left = "◢"
top_right = "◣"
bottom_left = "◥"
bottom_right = "◤"
vertical_left = "│"
vertical_right = "│"
horizontal_top = "═"
horizontal_bottom = "─"

[title]
style = "gradient"          # plain | badge | gradient | bracketed
case = "upper"
position = "top"
alignment = "left"
bold = true
brackets = ["┤ ", " ├"]

[bars]
style = "blocks"            # blocks | shade | ascii
show_value = true

[text]
bold_titles = true
dim_muted = true
italic_hints = false

[flourish]
grid_floor = true           # perspective grid in empty grid slots
sun = true
logo = "ansi-shadow"        # none | figlet-name
marquee = true
big_clock = { pixel = "quadrant" }   # full | half_height | quadrant | sextant | octant

[effects]
enabled = true
budget_ms = 4
startup      = { kind = "sweep_in", motion = "left_to_right", duration_ms = 600, gradient_len = 12 }
theme_swap   = { kind = "fade_from", duration_ms = 150 }
focus_change = { kind = "fade_from", duration_ms = 120, target = "border" }
alert_active = { kind = "hsl_pulse", lightness = 25, period_ms = 900, target = "crit_fg" }
ambient      = { crt_scanlines = false, crt_flicker = false, scanline_darken = 0.12 }

[components.gpu]            # per-component overrides: any of colors/gradients/glyphs/borders/title
gradients.power = ["$purple", "$pink", "$orange", "$red"]
colors.accent.primary = "$cyan"
[components.audio]
gradients.audio = ["$cyan", "$purple", "$pink"]
flourish.marquee = true
```
`modern.toml` is the same schema with `title.style="plain"`, `borders.set="rounded"`, `bars.style="blocks"`, `flourish.* = false`, `effects.startup/alert_active = none`, `text.dim_muted=false`.

## 4. Colour handling

- **Capability ladder** (resolved once at start, overridable by `--color=auto|always|never|16|256|truecolor` and config `color_mode`; per no-color.org, CLI flag/config beat `NO_COLOR`): `NO_COLOR` non-empty → mono; `COLORTERM` ∈ {truecolor, 24bit} → truecolor; `TERM` contains `256color` → 256; else 16. `supports-color 3.0.2` (`on(Stream::Stdout) -> Option<ColorLevel{has_basic,has_256,has_16m}>`) does this in one call.
- **crossterm 0.29 already honours NO_COLOR**: `Colored::ansi_color_disabled_memoized()` reads `NO_COLOR` once and the `Display` impl returns early, emitting *no* SGR colour at all (Rgb → `38;2;r;g;b`, indexed → `38;5;n`, no downsampling) — verified in `src/style/types/colored.rs`. So a truecolor theme under NO_COLOR silently becomes fg=bg=default and loses every gradient. The theme layer must therefore switch to a **mono theme** (all roles `Color::Reset`, meaning carried by `REVERSED/BOLD/DIM` + glyphs + shade ramps) exactly as astral-watch's `Theme{color:bool}` does. NO_COLOR says nothing about bold/underline, so modifiers stay.
- **256 downsampling**: do it yourself — `ansi_colours 1.2.3` is LGPL-3.0 (bad fit for an MIT crate); the algorithm is ~25 lines: nearest in the 6×6×6 cube (levels 0,95,135,175,215,255 → index 16+36r+6g+b) vs the 24-step grey ramp (8+10n → 232+n), squared-RGB distance. Precompute at theme load for every role and every gradient LUT entry; allow `[fallback16]`/`indexed:` overrides like gruvbox pairs each hex with an index. Computed examples: #ff2975→198, #00f0ff→51, #1e1e2e→235, #cdd6f4→189.
- **Contrast**: `palette 0.7.7` `Wcag21RelativeContrast` is implemented for `Srgb<f32>`: `fg.relative_contrast(bg)`, `has_min_contrast_text` (4.5), `has_min_contrast_large_text` (3), `has_enhanced_contrast_text` (7), `has_min_contrast_graphics` (3). Run at load: text/title vs bg,surface,panel ≥ 4.5; muted ≥ 4.5 (warn), ghost exempt; borders/severity-as-graphics ≥ 3; selection.fg vs selection.bg ≥ 4.5. `contrast.autofix = true` nudges L in Oklch toward text until it passes (this is what saves Rosé Pine Dawn's gold). Note WCAG 2.1 is "not entirely consistent with perceived contrast" (palette docs) — treat as a floor, not a target.
- **Colour-blind-safe severity**: colour + shape + text (`● OK`, `▲ WARN`, `■ CRIT`), crit additionally BOLD|REVERSED; never red/green-only; high-contrast theme uses Okabe-Ito; gradients that must read in greyscale should be luminance-monotonic (phosphor ramps are; retrowave's is not — that is a deliberate aesthetic choice and why the numeric value is always printed next to the bar).
- **Light variants**: same roles; DIM off; explicit `bg` painted over the whole frame (`Buffer::set_style(frame.area(), Style::new().bg(bg))` first thing in draw), otherwise the terminal's own background leaks through; `"reset"` bg for transparent themes.

## 5. Retro flourishes, cheaply and theme-gated

- **Gradient title**: `Line::from(title.chars().enumerate().map(|(i,c)| Span::styled(c, Style::new().fg(theme.gradient(Title).at(i as f32 / n)))))` → `Block::title_top(line)`. Cost: n spans, once per frame.
- **Sun & grid** in empty grid slots (or as a backdrop behind translucent-looking panels): a `Canvas` with `Marker::HalfBlock` drawing horizontal lines at y = h·(1 − 1/k) for k = 1.. (perspective spacing) and converging verticals, colour from `gradients.title`; the sun is stacked `▀/█` rows with 1-row gaps in the lower half, coloured by row from `gradients.load`. Precompute the string art per (w,h) and cache — zero per-frame maths. `flourish.grid_floor/sun` gate it; modern never draws it.
- **Big clock / big numbers**: `tui-big-text 0.8.9` (`BigText::builder().pixel_size(PixelSize::Quadrant).style(..).lines(vec![line]).centered().build()`; `PixelSize::{Full (8×8 cells/glyph), HalfHeight, HalfWidth, Quadrant (4×4), ThirdHeight, Sextant, QuarterHeight, Octant}`) — sextant/octant are OK on VTE (minifont) but keep `quadrant` as the default in shipped themes since quadrants are in DejaVu too. Footprint rule: 1×1 slot → plain text; ≥4 rows → Quadrant; ≥8 rows → Full.
- **ASCII/ANSI logo**: `const LOGO: &str` figlet "ANSI Shadow" render, coloured per row by gradient; shown on the startup sweep and in the 6×3 "hero" slot only.
- **Marquee** (Winamp component): shift a `Line` by an offset advanced every ~120 ms, wrap with a `  ***  ` separator; pause when the component is not visible.
- **Neon**: use `border_focused` + BOLD, and the alert `hsl_pulse` hook; "glow" is faked by a `text_ghost`-coloured second frame drawn one cell outside the focused panel border when the grid has gutters (`borders.glow = true`).
- Everything is behind `[flourish]`/`[effects]` booleans, so `modern` is just a theme file with them false — no code branches by theme name.

## 6. Hot reload and per-component overrides

- `notify` (9.0.0-rc.5 today; 8.2.x is the last stable — either works, API is the same for this use): `let mut w = notify::recommended_watcher(move |res: notify::Result<Event>| { let _ = tx.send(AppEvent::FsEvent(res)); })?; w.watch(themes_dir, RecursiveMode::NonRecursive)?;` — watch the **directory**, not the file, because editors save via atomic rename (notify docs call this out). Debounce 150 ms (`notify-debouncer-full 0.8.0-rc.2` pairs with notify 9; 0.7 with notify 8; or a hand-rolled "last event + 150 ms" timer). On event: parse+validate on a worker thread, then send `AppEvent::Theme(Arc<Theme>)`; on error keep the old theme and show a toast with the TOML error (line/col); on success swap the `Arc` and fire the `theme_swap` effect. Also watch `config.toml` for layout/`theme = "..."` changes. Config layering with `figment 0.10.19` (`Figment::new().merge(Serialized::defaults(Config::default())).merge(Toml::file(path)).merge(Env::prefixed("OPSTUI_").split("__")).extract()`, missing files are fine) is nice but pulls `toml ^0.8` next to `toml 1.1.4` — for a personal tool plain `toml` + `serde` and a 30-line merge is simpler.
- **Per-component overrides** are resolved at load, not per frame: `Theme::for_component(id) -> Arc<Theme>` returns a pre-merged clone (a Theme is a few hundred bytes plus gradient LUTs), stored in `HashMap<ComponentId, Arc<Theme>>`; a component's render context carries `&Theme`. Precedence: CLI/env > user theme `[components.x]` > user theme > parent (`inherits`) > built-in. Users can also point a component at a different theme entirely (`[components.audio] theme = "phosphor-amber"`).

## 7. Rust sketch

```rust
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::widgets::{Block, BorderType};
use palette::{IntoColor, Mix, Oklab, Srgb};
use palette::color_difference::Wcag21RelativeContrast;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::EnumIter, strum::EnumCount)]
pub enum Role { Bg, Surface, Panel, Border, BorderFocused, Title, Text, TextMuted, TextGhost,
    AccentPrimary, AccentSecondary, AccentTertiary, Ok, Warn, Crit, Info, SelectionFg, SelectionBg, Cursor }

#[derive(Clone, Copy)] pub enum ColorMode { TrueColor, Ansi256, Ansi16, Mono }
#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub enum GradientId { Load, Temp, Power, Mem, NetRx, NetTx, Audio, Title }

/// Precomputed perceptual gradient (Oklab), 64 steps, already downsampled for `mode`.
#[derive(Clone)]
pub struct Gradient { lut: [Color; 64], shade: [&'static str; 5] }
impl Gradient {
    pub fn build(stops: &[Srgb<u8>], mode: ColorMode) -> Self {
        let labs: Vec<Oklab> = stops.iter().map(|s| s.into_format::<f32>().into_color()).collect();
        let lut = std::array::from_fn(|i| {
            let t = i as f32 / 63.0 * (labs.len() - 1) as f32;
            let (k, f) = (t.floor() as usize, t.fract());
            let lab = if k + 1 < labs.len() { labs[k].mix(labs[k + 1], f) } else { labs[k] };
            let rgb: Srgb<f32> = lab.into_color();          // IntoColor clamps into gamut
            let rgb: Srgb<u8> = rgb.into_format();
            resolve(Color::Rgb(rgb.red, rgb.green, rgb.blue), mode)
        });
        Self { lut, shade: [" ", "░", "▒", "▓", "█"] }
    }
    pub fn at(&self, t: f32) -> Color { self.lut[((t.clamp(0.0, 1.0)) * 63.0) as usize] }
    /// Mono fallback: density instead of hue.
    pub fn shade_at(&self, t: f32) -> &'static str { self.shade[((t.clamp(0.0, 1.0)) * 4.0).round() as usize] }
}

#[derive(Clone)]
pub struct Theme {
    pub meta: Meta,
    colors: [Color; Role::COUNT],           // resolved for `mode`
    gradients: enum_map::EnumMap<GradientId, Gradient>,
    pub glyphs: Glyphs, pub borders: Borders, pub title: TitleSpec,
    pub flourish: Flourish, pub effects: Effects,
    pub mode: ColorMode,
}
impl Theme {
    pub fn color(&self, r: Role) -> Color { self.colors[r as usize] }
    pub fn style(&self, r: Role) -> Style { Style::new().fg(self.color(r)) }
    pub fn gradient(&self, g: GradientId) -> &Gradient { &self.gradients[g] }
    pub fn severity(&self, s: Severity) -> (Style, &str) {
        match (s, self.mode) {
            (Severity::Crit, ColorMode::Mono) => (Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED), self.glyphs.crit),
            (Severity::Crit, _) => (self.style(Role::Crit).add_modifier(Modifier::BOLD), self.glyphs.crit),
            (Severity::Warn, _) => (self.style(Role::Warn), self.glyphs.warn),
            (Severity::Ok, _)   => (self.style(Role::Ok), self.glyphs.ok),
        }
    }
    pub fn block<'a>(&self, title: &str, focused: bool) -> Block<'a> {
        let set: border::Set = if focused { self.borders.focused.set() } else { self.borders.normal.set() };
        let border_role = if focused { Role::BorderFocused } else { Role::Border };
        Block::bordered().border_set(set).border_style(self.style(border_role))
            .style(Style::new().bg(self.color(Role::Panel)))
            .title_top(self.title.render(title, self))    // Line: plain / badge / gradient / bracketed
    }
}

/// Theme file (serde) -> resolved Theme. `ThemeFile` mirrors the TOML; every colour is a String.
impl Theme {
    pub fn load(file: &ThemeFile, parent: Option<&ThemeFile>, mode: ColorMode) -> Result<Self, ThemeError> {
        let merged = file.merged_over(parent);
        let pal = |s: &str| -> Result<Srgb<u8>, ThemeError> { /* "$name" | "#rrggbb" | "#rgb" via Srgb::<u8>::from_str | "indexed:n" | "named:x" via Color::from_str | "reset" */ };
        let mut colors = [Color::Reset; Role::COUNT];
        for role in Role::iter() { colors[role as usize] = resolve(merged.colors.get(role, &pal)?, mode); }
        // contrast gate (truecolor/256 only): text roles vs bg/surface/panel >= 4.5, graphics >= 3.0
        for (fg, bg, min) in [(Role::Text, Role::Bg, 4.5), (Role::Title, Role::Panel, 4.5), (Role::TextMuted, Role::Panel, 4.5), (Role::Border, Role::Bg, 3.0)] {
            if let (Some(f), Some(b)) = (srgb_f32(merged.colors.raw(fg)), srgb_f32(merged.colors.raw(bg))) {
                let ratio = f.relative_contrast(b);
                if ratio < min { tracing::warn!(?fg, ?bg, ratio, min, "theme contrast below threshold"); }
            }
        }
        Ok(Self { /* … gradients via Gradient::build, glyph/border/title/effect specs … */ })
    }
}

/// Capability downsampling. Mono => Reset (meaning must come from modifiers/glyphs).
pub fn resolve(c: Color, mode: ColorMode) -> Color {
    match (c, mode) {
        (_, ColorMode::Mono) => Color::Reset,
        (Color::Rgb(r, g, b), ColorMode::Ansi256) => Color::Indexed(nearest_256(r, g, b)),
        (Color::Rgb(r, g, b), ColorMode::Ansi16) => nearest_16(r, g, b),
        (c, _) => c,
    }
}
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const L: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let q = |v: u8| L.iter().enumerate().min_by_key(|(_, &l)| (v as i32 - l as i32).abs()).map(|(i, _)| i as u8).unwrap();
    let (ri, gi, bi) = (q(r), q(g), q(b));
    let cube = (16 + 36 * ri + 6 * gi + bi, [L[ri as usize], L[gi as usize], L[bi as usize]]);
    let gray_n = (((r as u32 + g as u32 + b as u32) / 3).saturating_sub(8) / 10).min(23) as u8;
    let gv = 8 + 10 * gray_n;
    let d = |c: [u8; 3]| (r as i32 - c[0] as i32).pow(2) + (g as i32 - c[1] as i32).pow(2) + (b as i32 - c[2] as i32).pow(2);
    if d([gv, gv, gv]) < d(cube.1) { 232 + gray_n } else { cube.0 }
}

pub fn detect_mode(cli: Option<ColorMode>) -> ColorMode {
    if let Some(m) = cli { return m; }                                  // flags beat NO_COLOR (no-color.org)
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) { return ColorMode::Mono; }
    match supports_color::on(supports_color::Stream::Stdout) {
        Some(l) if l.has_16m => ColorMode::TrueColor,
        Some(l) if l.has_256 => ColorMode::Ansi256,
        Some(_) => ColorMode::Ansi16,
        None => ColorMode::Mono,
    }
}
```
Hot-reload thread (sketch): `recommended_watcher` → `mpsc` → debounce 150 ms → `Theme::load` → `AppEvent::Theme(Arc<Theme>)`; the app keeps `Arc<Theme>` + `HashMap<ComponentId, Arc<Theme>>` and, on swap, `effects.add_unique_effect("swap", fx::fade_from(old.color(Role::Text), old.color(Role::Bg), 150))`.

**Cargo features to enable:** `ratatui = { version = "0.30.2", features = ["serde", "palette"] }` (palette feature only adds `Color::from_hsl/from_hsluv`; the real work uses `palette` directly), `palette = { version = "0.7.7", features = ["std"] }` (MSRV 1.71), `tachyonfx = "0.25.1"` (MSRV not declared; ratatui-core 0.1.2 needs 1.88), `tui-big-text = "0.8.9"` (MSRV 1.88), `notify` + optional `notify-debouncer-full`, `serde 1.0.229`, `toml 1.1.4`, `supports-color 3.0.2`. No extra apt packages or privileges are needed for any of this; everything is pure Rust and reads only env vars and the terminal.

## Recommendations

- **Model themes as semantic roles + named gradients + glyph/border/title/effect specs, never raw colours in components; ship the schema as TOML with `[palette]` indirection (`$name`), `inherits`, and `[components.<id>]` overrides.** — This is the only way one render path yields both a loud retrowave and a clean modern look; btop/bottom/zellij all converge on role-based keys, and zellij's per-component style bundles are the proven pattern for overrides.
  - alternatives: btop-style flat key list (no gradients beyond 3 stops, no glyph/effect layer); base16 (16 slots too few for gradients and severity+accents).
- **Make `modern` = Catppuccin Mocha verbatim (Base #1e1e2e, Text #cdd6f4, Surface1 border, Lavender focus, Green/Yellow/Peach/Red gradient) and derive `retrowave` from Synthwave '84 + #ff2975, with the computed contrast fixes (border #7a3fb5, muted #8a7fb0).** — Catppuccin's neutrals pass AA everywhere I checked (Text 11.3:1, Subtext0 7.4:1); Synthwave '84 values are the widely recognised retro reference, and the computed ratios show which pairs need adjustment.
  - alternatives: Nord (nord3 muted fails at 1.69:1, needs DIM tricks), Tokyo Night (muted #565f89 is 2.76:1), Rosé Pine (great, but its gold fails on the light variant).
- **Resolve colour capability once (CLI > config > NO_COLOR > COLORTERM > TERM), precompute downsampled role colours and 64-entry Oklab gradient LUTs at theme load, and switch to a dedicated mono theme (Reset colours + REVERSED/BOLD/DIM + shade ramps ` ░▒▓█`) under NO_COLOR.** — crossterm 0.29 emits no colour SGR at all when NO_COLOR is set (verified), so without a mono theme every gradient disappears; per-frame colour maths is unnecessary once LUTs exist.
  - alternatives: Per-frame palette conversion (wasteful); relying on crossterm alone (loses all meaning under NO_COLOR).
- **Write your own ~25-line nearest-xterm-256 function; do not depend on `ansi_colours`.** — ansi_colours 1.2.3 is LGPL-3.0-or-later, awkward for an MIT crate; the cube+grey algorithm is trivial and the computed indices (e.g. #ff2975→198, #00f0ff→51) are good enough.
  - alternatives: ansi_colours (LGPL), termcolor-style 16-colour only.
- **Validate contrast at theme load with palette's `Wcag21RelativeContrast` (text ≥4.5:1, graphics/borders ≥3:1) and offer `contrast.autofix` that darkens/lightens in Oklch until it passes; log warnings, never refuse to load.** — Light themes (Rosé Pine Dawn gold at 2.05:1) and several dark 'muted' colours fail AA; a load-time gate catches this for user-authored themes without blocking hot reload.
  - alternatives: No checks (silent unreadable themes); hard failure (breaks hot-reload iteration).
- **Severity is always colour + glyph + text (● OK / ▲ WARN / ■ CRIT), crit adds BOLD|REVERSED; the high-contrast theme uses the Okabe-Ito palette.** — Colour-blind safety and NO_COLOR/mono both need a non-colour channel; Okabe-Ito is the standard vetted set.
  - alternatives: Colour-only severity (fails deuteranopia and NO_COLOR).
- **Glyph tiers ascii / unicode / nerd with `nerd = false` default and the unicode tier restricted to U+2500–259F, U+25xx shapes and braille; treat sextants/octants as 'VTE-synthesised, optional' and keep quadrant big-text as the shipped default.** — Ubuntu Sans Mono (the actual Ptyxis font here) lacks rounded corners, eighths, quadrants and braille, but VTE's minifont draws all of U+2500–259F and the legacy-computing ranges itself; braille is the only glyph family relying on font fallback; no nerd font is installed.
  - alternatives: Requiring a Nerd Font (user explicitly does not want that by default).
- **Effects via tachyonfx 0.25.1 as named hooks (startup sweep_in, theme_swap fade_from, focus fade, alert hsl_shift ping-pong pulse, optional CRT effect_fn), bounded to the affected panel, ≤600 ms for event effects, ambient CRT off by default even in retrowave, a `budget_ms` watchdog that auto-disables ambient effects, and redraw only when `EffectManager::is_running() || dirty`.** — tachyonfx is O(cells) per effect per frame and full-screen bg changes inflate the terminal diff to ~100 KB/frame; bounding areas and durations keeps the 'showcase' feel without turning a monitor into a CPU hog.
  - alternatives: Hand-rolled animation (reinventing easing/timers); always-on CRT (costly, and defeats 'modern').
- **Hot reload with `notify` watching the themes *directory* (NonRecursive) plus 150 ms debounce, parse on a worker thread, swap an `Arc<Theme>` (and per-component pre-merged `Arc<Theme>`s) through the app event channel, keep the old theme on parse error and toast the message.** — Editors save via atomic rename so file-level watches go stale (notify docs); pre-merging per-component overrides keeps the render loop allocation-free.
  - alternatives: Polling every second (fine but laggy); figment for layering (pulls toml 0.8 next to toml 1.x).
- **Ship an `opstui theme import` for alacritty/wezterm 16-ANSI schemes and base16/base24 YAML with a fixed role mapping (bg=primary.background, text=foreground, muted=bright.black, ok/warn/crit=green/yellow/red, accents=blue/magenta/cyan, selection/cursor from their sections; base00→bg … base0D→primary).** — Hundreds of ready-made schemes exist in those formats; a mapper makes 'and others' cheap and demonstrates the schema's generality.
  - alternatives: Hand-porting each scheme.

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `ratatui` | 0.30.2 | TUI framework; enable features `serde` (Color (de)serialises as "#RRGGBB"/name/index strings) and optionally `palette` (Color::from_hsl/from_hsluv). Default features: all-widgets, crossterm, layout-cache, macros, underline-color. MSRV 1.88. | none | verified |
| `crossterm` | 0.29.0 | Backend (pulled by ratatui's `crossterm` feature). Emits 38;2;r;g;b for Rgb, 38;5;n for indexed; suppresses ALL colour when NO_COLOR is non-empty; does no downsampling. | none | verified |
| `palette` | 0.7.7 | Hex parsing (`Srgb<u8>: FromStr`, accepts #rgb/#rrggbb with or without #), Oklab/Oklch mixing (`Mix`, `IntoColor`), WCAG 2.1 contrast (`Wcag21RelativeContrast` for Srgb<f32>). Features: std (default-ish), serde optional. MSRV 1.71. | none | verified |
| `tachyonfx` | 0.25.1 | Shader-like effects on ratatui buffers: fx::{sweep_in, fade_from, fade_to, hsl_shift, coalesce, dissolve, slide_in, ping_pong, repeating, never_complete, parallel, sequence, delay, effect_fn}, EffectManager<K>, CellFilter, Interpolation (34 easings), Motion. Depends on ratatui-core ^0.1.2 (ratatui 0.30-compatible). Features: std, std-duration, sendable, dsl (DslCompiler), ratatui-next-cell. MSRV not declared. | none | verified |
| `tui-big-text` | 0.8.9 | Big clock / big numbers via font8x8: BigText::builder().pixel_size(PixelSize::{Full,HalfHeight,HalfWidth,Quadrant,ThirdHeight,Sextant,QuarterHeight,Octant}).style().lines().centered().build(). Depends on ratatui-core ^0.1 / ratatui-widgets ^0.3. MSRV 1.88. | none | verified |
| `notify` | 9.0.0-rc.5 (or 8.2.x stable) | Theme/config hot reload: recommended_watcher(FnMut(Result<Event>)) + watch(dir, RecursiveMode::NonRecursive). inotify on Linux. CC0 licence. MSRV 1.88 for 9.x. | none (inotify; fs.inotify.max_user_watches only matters for recursive watches) | verified |
| `notify-debouncer-full` | 0.8.0-rc.2 (pairs with notify 9); 0.7.0 pairs with notify ^8.2 | Optional: new_debouncer(timeout, tick_rate, handler) merges rename/modify bursts from editors' atomic saves. | none | verified |
| `serde` | 1.0.229 | Theme/config (de)serialisation with derive. | none | verified |
| `toml` | 1.1.4 | Parse theme files (error messages with line/col for the reload toast). | none | likely |
| `supports-color` | 3.0.2 | One-call capability detection: on(Stream::Stdout) -> Option<ColorLevel{has_basic,has_256,has_16m}>, honours NO_COLOR. MSRV 1.70. | none | verified |
| `figment` | 0.10.19 | Optional layered config (defaults + TOML file + OPSTUI_ env). Note it depends on toml ^0.8, duplicating toml 1.x if both are used. | none | verified |
| `ansi_colours` | 1.2.3 | RGB→256 approximation — NOT recommended: LGPL-3.0-or-later. Write the ~25-line nearest-colour function instead. | none | verified |
| `enum-map / strum` | latest | Role/GradientId indexed arrays and iteration for the Theme struct (convenience only). | none | uncertain |

## Risks

- **NO_COLOR makes crossterm drop every colour escape, so a truecolor theme silently loses gradients, bars and severity colouring.** → Detect NO_COLOR in the theme layer and load the mono theme (Reset colours, REVERSED/BOLD/DIM, shade ramps, severity glyphs); also expose --color=always to override per no-color.org precedence.
- **Ptyxis here renders with Ubuntu Sans Mono (GNOME monospace setting), which lacks rounded corners, eighth/quadrant blocks, braille and shapes; a user who switches to a non-VTE terminal (e.g. a tty or ssh from elsewhere) loses VTE's built-in glyph synthesis.** → Keep the unicode tier to U+2500–259F + a few U+25xx shapes, provide the ascii tier and `--glyphs ascii`, treat sextant/octant as opt-in, smoke-test braille visually once.
- **Ambient tachyonfx effects (CRT scanlines/flicker) touch half the screen per frame, ballooning the terminal diff and CPU/pty traffic; alert pulses that run forever keep the redraw loop hot.** → Ambient effects off by default, `effects.budget_ms` watchdog with auto-disable, bounded areas via with_area/CellFilter, `is_running() || dirty` redraw gating, `--no-effects`.
- **Several popular palettes fail WCAG AA for 'muted' text (Nord nord3 1.69:1, Tokyo Night #565f89 2.76:1, Rosé Pine Dawn gold 2.05:1), so faithful ports look unreadable in small panels.** → Load-time contrast gate with warnings, `text_ghost` role for decorative-only colours, optional Oklch autofix, always print numeric values next to colour-coded bars.
- **Hot reload races: editors write via temp+rename, producing Create/Rename/Modify bursts and half-written files.** → Watch the directory, debounce ≥150 ms, parse on a worker thread, keep the last good theme on error, show the parse error as a toast.
- **Pre-release dependencies (notify 9.0.0-rc.5, notify-debouncer-full 0.8.0-rc.2) may change API before stable; tachyonfx moves fast (0.25 in mid-2026) and its MSRV is undeclared.** → Pin exact versions in Cargo.lock, keep the effect layer behind a small internal trait so a tachyonfx bump is a one-file change, or use notify 8.2 + debouncer 0.7 until 9.0 stabilises.
- **Duplicate `toml` majors (figment→toml 0.8, direct toml 1.1) and licence mismatch (ansi_colours LGPL) can sneak into the dependency graph.** → Skip figment unless profiles are needed; write the 256-colour mapper in-tree; add `cargo deny` licence check to CI (fits the astral-watch CI conventions).
- **Themes that set an explicit `bg` but do not paint the full frame leak the terminal background through gutters; transparent themes (`bg = "reset"`) look wrong under panels with explicit `panel` colours.** → Paint `Buffer::set_style(frame.area(), bg)` first every frame; validate at load that `panel`/`surface` are not Reset when `bg` is, or vice versa.

## Verified facts

- Local: TERM=xterm-256color, COLORTERM=truecolor, NO_COLOR unset, VTE_VERSION=8400; `tput colors`=256; terminfo has colors#0x100 and no Tc/RGB capability (infocmp -x xterm-256color).
- Local: libvte-2.91-0 0.84.0-2 and ptyxis 50.1-1ubuntu2 installed (dpkg -l); gsettings org.gnome.Ptyxis use-system-font=true, org.gnome.desktop.interface monospace-font-name='Ubuntu Sans Mono 11', color-scheme='prefer-dark' — so the terminal font is Ubuntu Sans Mono, although `fc-match monospace` returns DejaVu Sans Mono.
- Local (fc-list :charset=…): Ubuntu Sans Mono lacks U+2570, U+257F, U+2594, U+2595, U+259F, U+2800, U+25E2, U+25CF, U+25B6, U+1FB00, U+E0B0; has U+2500, U+2588, U+2591. DejaVu Sans Mono covers U+2500–259F, U+25E2, U+25CF, U+25B6, U+2764 but not U+2800 (fallback DejaVu Sans), not U+1FB00 (fallback Noto Sans Symbols2), not U+E0B0 (only OpenSymbol). No installed font covers octants U+1CD00. Installed monospace families: DejaVu Sans Mono, Liberation Mono, Nimbus Mono PS, Noto Mono, Ubuntu Mono, Ubuntu Sans Mono.
- VTE master src/minifont.cc draws U+2500–257F, U+2580–259F, U+2571–2573, U+23B8–23BD, U+25E2–25E5, U+1FB00–1FBBF, U+1CD00–1CDE5, U+1CE90–1CEAF, U+1CC21–1CC2F, U+1CE51–1CE8F itself (WebFetch of raw.githubusercontent.com/GNOME/vte/master/src/minifont.cc).
- ratatui 0.30.2 (cargo info): rust-version 1.88.0; default features all-widgets, crossterm, layout-cache, macros, underline-color; optional palette (palette ^0.7.6 with libm), serde, crossterm_0_28/0_29, scrolling-regions, unstable-*. Depends on ratatui-core ^0.1.2, ratatui-widgets ^0.3.2, ratatui-crossterm ^0.1.2 (crates.io API).
- ratatui Color (docs.rs 0.30.2 + ratatui-core color.rs source): 19 variants incl. Rgb(u8,u8,u8)/Indexed(u8); FromStr parses names (with bright/light/grey aliases, separators ignored), u8 indices, and 7-char #RRGGBB; Display prints #RRGGBB (uppercase) / decimal index; serde uses those strings (plus legacy tagged maps on deserialize); `const fn from_u32(0x00RRGGBB)`; `from_hsl(palette::Hsl)`/`from_hsluv` behind `palette` feature; no From<palette::Srgb>.
- ratatui 0.30 breaking changes (BREAKING-CHANGES.md): block::Title removed, Block::title takes Into<Line>; Style no longer implements Stylize; Marker is #[non_exhaustive]; layout::Alignment → HorizontalAlignment alias; MSRV 1.86 then 1.88 in 0.30.1.
- ratatui Modifier constants: BOLD, DIM, ITALIC, UNDERLINED, SLOW_BLINK, RAPID_BLINK, REVERSED, HIDDEN, CROSSED_OUT (docs.rs 0.30.2). style::palette has tailwind (Palette c50..c950, 24 constants e.g. RED.c500 == Rgb(239,68,68)) and material.
- ratatui symbols::border::Set fields: top_left, top_right, bottom_left, bottom_right, vertical_left, vertical_right, horizontal_top, horizontal_bottom (cached ratatui-0.29.0 source; unchanged per 0.30 docs). Constants: PLAIN, ROUNDED, DOUBLE, THICK, FULL, EMPTY, LIGHT/HEAVY_{DOUBLE,TRIPLE,QUADRUPLE}_DASHED, ONE_EIGHTH_TALL/WIDE, PROPORTIONAL_TALL/WIDE, QUADRANT_BLOCK/INSIDE/OUTSIDE. BorderType variants: Plain, Rounded, Double, Thick, Light/HeavyDoubleDashed, Light/HeavyTripleDashed, Light/HeavyQuadrupleDashed, QuadrantInside, QuadrantOutside (docs.rs 0.30.2).
- ratatui 0.30.2 Block methods: title, title_top, title_bottom, title_alignment, title_style, title_position, borders, border_style, border_type, border_set, style, padding, merge_borders(MergeStrategy::{Replace, Exact, Fuzzy}), inner (docs.rs). symbols modules include bar (NINE_LEVELS/THREE_LEVELS), line, shade (EMPTY LIGHT MEDIUM DARK FULL), pixel (QUADRANTS, SEXTANTS, OCTANTS), merge; Marker::{Dot, Block, Bar, Braille, HalfBlock}.
- ratatui widgets: Sparkline::data accepts u64 / Option<u64> / SparklineBar (per-bar style), direction(RenderDirection::{LeftToRight,RightToLeft}), absent_value_style/symbol; Gauge::use_unicode gives 8 sub-cell steps, gauge_style/label/ratio/percent; LineGauge filled_style/unfilled_style/filled_symbol/unfilled_symbol (line_set and gauge_style deprecated); BarChart bar_set/bar_style/value_style/label_style/direction, Bar::style/value_style/text_value; GraphType::{Scatter, Line, Bar, Area (fill_to_y)}; Buffer::cell/cell_mut return Option, set_style(area, style), set_string/set_span/set_line (docs.rs 0.30.2).
- tachyonfx 0.25.1 (cargo info + crates.io API + docs.rs): MIT, features dsl/std/std-duration/sendable/wasm/ratatui-next-cell; depends on ratatui-core ^0.1.2; EffectRenderer::render_effect(&mut self, effect: &mut Effect, area: Rect, last_tick: Duration) implemented for Frame and Buffer; EffectManager<K>: unique, add_effect, add_unique_effect, cancel_unique_effect, is_running, process_effects(duration, buf, area); Effect: with_area, with_filter(CellFilter), with_pattern, with_color_space, with_rng, done, running, reset, area, set_area, timer, process; Duration = custom struct { milliseconds: u32 }; EffectTimer From<u32>, From<(u32, Interpolation)>, From<Duration>, From<(Duration, Interpolation)>; 34 Interpolation variants; Motion::{LeftToRight, RightToLeft, UpToDown, DownToUp}; CellFilter variants All, Area, RefArea, FgColor, BgColor, Inner, Outer, Text, NonEmpty, AllOf, AnyOf, NoneOf, Not, Layout, PositionFn, EvalCell, Static.
- tachyonfx fx signatures (docs.rs): fade_from<T: Into<EffectTimer>, C: Into<Color>>(fg: C, bg: C, timer: T); hsl_shift<T>(hsl_fg_change: Option<[f32;3]>, hsl_bg_change: Option<[f32;3]>, timer) (panics if both None); sweep_in<T, C>(direction: Motion, gradient_length: u16, randomness: u16, faded_color: C, timer: T); effect_fn<F,S,T>(state: S, timer: T, f: FnMut(&mut S, ShaderFnContext, CellIterator)); plus coalesce, dissolve, slide_in/out, ping_pong, repeating, repeat, never_complete, parallel, sequence, delay, with_duration, prolong_start/end, offscreen_buffer, dynamic_area, translate, resize_area, etc.
- tui-big-text 0.8.9 (cargo info, crates.io API, docs.rs): MSRV 1.88.0, depends on ratatui-core ^0.1, ratatui-widgets ^0.3, font8x8 ^0.3; BigText::builder().pixel_size(PixelSize::{Full, HalfHeight, HalfWidth, Quadrant, ThirdHeight, Sextant, QuarterHeight, Octant}).style(..).lines(vec![..]).centered().build().
- palette 0.7.7 (cargo info + docs.rs): MSRV 1.71, features std/named/named_from_str/serde; Rgb<S,u8> implements FromStr for '#f8b', '#ff88bb', with/without '#'; Rgb implements Mix; into_format/from_format u8<->f32; IntoColor to Oklab/Oklch/Lch; Wcag21RelativeContrast (relative_luminance, relative_contrast, has_min_contrast_text, has_min_contrast_large_text, has_min_contrast_graphics, has_enhanced_contrast_text, has_enhanced_contrast_large_text) implemented for Rgb<S: RgbStandard<Space=Srgb>, T>.
- crossterm 0.29 src/style/types/colored.rs: ansi_color_disabled() = NO_COLOR non-empty; memoized with Once; Display returns Ok(()) early (emits nothing) when disabled; Rgb written as 2;r;g;b and AnsiValue as 5;n after 38;/48;.
- notify 9.0.0-rc.5 (cargo info + docs.rs): CC0-1.0, MSRV 1.88, features tokio/crossbeam-channel/flume/futures/serde; recommended_watcher<F: EventHandler>(F) -> Result<RecommendedWatcher>, EventHandler accepts FnMut(Result<Event>) and mpsc::Sender; docs warn editors replace files atomically → watch parent directory; inotify max_user_watches note. notify-debouncer-full 0.8.0-rc.2 depends on notify ^9.0.0-rc.4; 0.7.0 depends on notify ^8.2.0.
- figment 0.10.19 (cargo info + crates.io API + docs.rs): features toml/env/json/yaml; depends on toml ^0.8; Figment::new().merge(Toml::file(..)).merge(Env::prefixed(..)).extract(); Toml::file tolerates missing files; merge replaces, join fills gaps.
- supports-color 3.0.2 (cargo info + docs.rs): on(Stream)/on_cached -> Option<ColorLevel{level, has_basic, has_256, has_16m}>, honours NO_COLOR; MSRV 1.70. ansi_colours 1.2.3 is LGPL-3.0-or-later (cargo info).
- no-color.org: disable colour when NO_COLOR is present and non-empty; covers only colour (not bold/underline); user config and CLI flags should override NO_COLOR.
- btop theme keys (themes/dracula.theme on GitHub): main_bg, main_fg, title, hi_fg, selected_bg, selected_fg, inactive_fg, graph_text, meter_bg, proc_misc, cpu_box, mem_box, net_box, proc_box, div_line, and temp/cpu/free/cached/available/used/download/upload/process _start/_mid/_end.
- bottom [styles] keys (clementtsang.github.io/bottom): theme; widgets.{border_colour, selected_border_colour, widget_title, bg_colour, text, selected_text, disabled_text, thread_text}; graphs.{graph_colour, legend_text}; cpu.{all_entry_colour, avg_entry_colour, cpu_core_colours}; memory.{ram_colour, cache_colour, swap_colour, arc_colour, gpu_colours}; network.{rx_colour, tx_colour, rx_total_colour, tx_total_colour}; battery.{high,medium,low}_battery_colour; tables.headers; values '#hex' | 'r, g, b' | names | { colour, bg_colour, bold }.
- zellij theme KDL (zellij.dev/documentation/themes + assets/themes/dracula.kdl): components text_unselected, text_selected, ribbon_unselected, ribbon_selected, table_title, table_cell_unselected, table_cell_selected, list_unselected, list_selected, frame_unselected, frame_selected, frame_highlight, exit_code_success, exit_code_error, each with base/background/emphasis_0..3 RGB triplets; multiplayer_user_colors player_1..player_10.
- alacritty theme TOML (rose-pine/alacritty dist and alacritty-theme dracula.toml): [colors.primary] background/foreground/dim_foreground/bright_foreground, [colors.cursor] text/cursor, [colors.vi_mode_cursor], [colors.search.matches|focused_match], [colors.hints.start|end], [colors.line_indicator], [colors.footer_bar], [colors.selection] text/background, [colors.normal|bright|dim] black..white. wezterm colors keys (wezterm.org/config/appearance): foreground, background, cursor_bg, cursor_fg, cursor_border, selection_fg, selection_bg, scrollbar_thumb, split, ansi, brights, indexed, compose_cursor, visual_bell, copy_mode_*, quick_select_*, tab_bar.*.
- base16 styling guide (tinted-theming/home): base00–07 tonal ramp dark→light, base08 red (variables, diff deleted), base09 orange (integers/constants), base0A yellow (classes, search bg), base0B green (strings, diff inserted), base0C cyan (support/regex), base0D blue (functions/headings), base0E magenta (keywords), base0F brown (deprecated); YAML keys system/name/author/variant/palette. base24 (tinted-theming/base24): base10 darker background, base11 darkest background, base12–17 bright red/yellow/green/cyan/blue/magenta with an explicit ANSI mapping.
- Catppuccin Mocha hex values (catppuccin.com/palette): Rosewater #f5e0dc, Flamingo #f2cdcd, Pink #f5c2e7, Mauve #cba6f7, Red #f38ba8, Maroon #eba0ac, Peach #fab387, Yellow #f9e2af, Green #a6e3a1, Teal #94e2d5, Sky #89dceb, Sapphire #74c7ec, Blue #89b4fa, Lavender #b4befe, Text #cdd6f4, Subtext1 #bac2de, Subtext0 #a6adc8, Overlay2 #9399b2, Overlay1 #7f849c, Overlay0 #6c7086, Surface2 #585b70, Surface1 #45475a, Surface0 #313244, Base #1e1e2e, Mantle #181825, Crust #11111b.
- Nord (nordtheme.com): nord0 #2e3440 … nord3 #4c566a, nord4 #d8dee9, nord5 #e5e9f0, nord6 #eceff4, nord7 #8fbcbb, nord8 #88c0d0, nord9 #81a1c1, nord10 #5e81ac, nord11 #bf616a (errors), nord12 #d08770, nord13 #ebcb8b (warnings), nord14 #a3be8c (success), nord15 #b48ead. Tokyo Night (folke/tokyonight.nvim alacritty extra): bg #1a1b26, fg #c0caf5, normal black #15161e red #f7768e green #9ece6a yellow #e0af68 blue #7aa2f7 magenta #bb9af7 cyan #7dcfff white #a9b1d6, bright black #414868. Rosé Pine (rose-pine/palette palette.json + alacritty dist): base #191724, surface #1f1d2e, overlay #26233a, muted #6e6a86, subtle #908caa, text #e0def4, love #eb6f92, gold #f6c177, rose #ebbcba, pine #31748f, foam #9ccfd8, iris #c4a7e7, selection bg #403d52, cursor #524f67; Dawn base #faf4ed, surface #fffaf3, overlay #f2e9e1, muted #9893a5, subtle #797593, text #464261, love #b4637a, gold #ea9d34, rose #d7827e, pine #286983, foam #56949f, iris #907aa9. Gruvbox (gruvbox.vim): dark0_hard #1d2021(234), dark0 #282828(235), dark0_soft #32302f(236), dark1 #3c3836(237), dark2 #504945(239), dark3 #665c54(241), dark4 #7c6f64(243), light0 #fbf1c7(229), light1 #ebdbb2(223), gray #928374(245), bright red #fb4934(167) green #b8bb26(142) yellow #fabd2f(214) blue #83a598(109) purple #d3869b(175) aqua #8ec07c(108) orange #fe8019(208), neutral red #cc241d … orange #d65d0e, faded red #9d0006 green #79740e yellow #b57614 blue #076678 purple #8f3f71 aqua #427b58 orange #af3a03. Dracula (draculatheme.com): Background #282a36, Current Line/Selection #44475a, Foreground #f8f8f2, Comment #6272a4, Cyan #8be9fd, Green #50fa7b, Orange #ffb86c, Pink #ff79c6, Purple #bd93f9, Red #ff5555, Yellow #f1fa8c. Synthwave '84 (robb0wen/synthwave-vscode): editor bg #262335, sidebar #241b2f, activity bar #171520, pink #ff7edb, cyan #03edf9, purple #9d8bca, orange #ff8b39, yellow #fede5d, red #fe4450, tab border #880088. Okabe-Ito (jfly.uni-koeln.de/color): #000000, #e69f00, #56b4e9, #009e73, #f0e442, #0072b2, #d55e00, #cc79a7.
- WCAG contrast ratios computed locally (python, WCAG 2.1 formula; sanity white/black = 21.0): retrowave text #efe9ff/#0b0324 16.9, muted #8a7fb0 5.46, pink #ff2975 5.53, crit #fe4450 5.85, border #5b2a86 2.01 (fail); Mocha text 11.34, Subtext0 7.37, Overlay1 4.44, Lavender 9.17, Red 7.08, Surface1 border 1.80; gruvbox-dark text 10.75, gray 4.02, crit 4.29; dracula text 13.36, comment 3.03, crit 4.53; nord nord3 1.69, nord4 9.25, crit 3.05; tokyo-night muted 2.76; rosé-pine-dawn text #464261 8.69, subtle 4.02, muted 2.73, gold 2.05, love 3.84, pine 5.59; gruvbox-light text 10.22, warn 3.33, ok 4.29; high-contrast Okabe-Ito on black: #009e73 6.14, #e69f00 9.32, #d55e00 5.43, #56b4e9 9.10. Nearest xterm-256 indices computed locally: #ff2975→198, #00f0ff→51, #b967ff→135, #ff8b39→209, #0b0324→233, #1e1e2e→235, #cdd6f4→189, #282828→235, #282a36→236.
- astral-watch (local src/tui.rs): Theme is `struct Theme { color: bool }` with `c(Color)->Color` collapsing to Color::Reset under NO_COLOR and `badge()` using REVERSED|BOLD; per-pin colours are named ratatui colours; ratatui 0.29 optional `tui` feature; MSRV 1.85 (1.88 with tui).

## Open questions

- Which VTE release first drew octants (U+1CD00–1CDE5) and sixteenths: master minifont.cc has them and 0.84 is current, but the 0.84 NEWS could not be fetched (404 on both GitLab raw and GitHub mirror paths) — a 30-second visual test in Ptyxis (print a sextant and an octant) settles whether PixelSize::Sextant/Octant can be default-safe.
- Exact units for tachyonfx::fx::hsl_shift's [h, s, l] deltas (degrees / percentage points vs 0..1) — verify on the docs page before tuning the alert pulse.
- tachyonfx `dsl` feature (DslCompiler) syntax and stability — attractive for letting theme files carry effect expressions as strings, but unverified; start with the typed `{ kind = … }` tables and add DSL passthrough later if it proves stable.
- Whether palette's IntoColor clamps out-of-gamut Oklab→sRGB results automatically (there is an IntoColorUnclamped counterpart, which suggests it does) — confirm or add an explicit Clamp step in Gradient::build.
- Rosé Pine Dawn text colour: palette.json fetched today says #464261 while older documentation lists #575279 — pick one when porting.
- Stable vs rc: whether to ship on notify 9.0.0-rc.5 (+ debouncer 0.8.0-rc.2) or notify 8.2.x (+ debouncer 0.7.0) until 9.0 is final.
- Is Ptyxis's own palette/profile (Ptyxis lets the user pick a terminal palette) relevant for a 'terminal-native' theme that uses Color::Reset/named colours only? Worth a `terminal` theme that maps roles to the 16 ANSI colours so it follows whatever Ptyxis palette is active.
- Braille rendering quality with Ubuntu Sans Mono + DejaVu Sans fallback (cell width/alignment) — needs a visual check; if it looks off, default `chart_marker` to `half_block` for this machine's profile.

## Sources

- https://docs.rs/ratatui/0.30.2/ratatui/style/enum.Color.html
- https://raw.githubusercontent.com/ratatui/ratatui/main/ratatui-core/src/style/color.rs
- https://raw.githubusercontent.com/ratatui/ratatui/main/BREAKING-CHANGES.md
- https://docs.rs/ratatui/0.30.2/ratatui/style/struct.Modifier.html
- https://docs.rs/ratatui/0.30.2/ratatui/style/palette/tailwind/index.html
- https://docs.rs/ratatui/0.30.2/ratatui/symbols/border/index.html
- https://docs.rs/ratatui/0.30.2/ratatui/symbols/index.html
- https://docs.rs/ratatui/0.30.2/ratatui/symbols/pixel/index.html
- https://docs.rs/ratatui/0.30.2/ratatui/symbols/merge/enum.MergeStrategy.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Block.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/enum.BorderType.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Sparkline.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Gauge.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.LineGauge.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.BarChart.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/enum.GraphType.html
- https://docs.rs/ratatui/0.30.2/ratatui/buffer/struct.Buffer.html
- https://docs.rs/tachyonfx/latest/tachyonfx/
- https://docs.rs/tachyonfx/latest/tachyonfx/all.html
- https://docs.rs/tachyonfx/latest/tachyonfx/fx/index.html
- https://docs.rs/tachyonfx/latest/tachyonfx/fx/fn.fade_from.html
- https://docs.rs/tachyonfx/latest/tachyonfx/fx/fn.hsl_shift.html
- https://docs.rs/tachyonfx/latest/tachyonfx/fx/fn.sweep_in.html
- https://docs.rs/tachyonfx/latest/tachyonfx/fx/fn.effect_fn.html
- https://docs.rs/tachyonfx/latest/tachyonfx/trait.EffectRenderer.html
- https://docs.rs/tachyonfx/latest/tachyonfx/struct.Effect.html
- https://docs.rs/tachyonfx/latest/tachyonfx/struct.EffectManager.html
- https://docs.rs/tachyonfx/latest/tachyonfx/struct.EffectTimer.html
- https://docs.rs/tachyonfx/latest/tachyonfx/enum.CellFilter.html
- https://docs.rs/tachyonfx/latest/tachyonfx/enum.Interpolation.html
- https://docs.rs/tachyonfx/latest/tachyonfx/enum.Motion.html
- https://docs.rs/tachyonfx/latest/tachyonfx/type.Duration.html
- https://docs.rs/tui-big-text/latest/tui_big_text/
- https://docs.rs/palette/0.7.7/palette/rgb/struct.Rgb.html
- https://docs.rs/palette/0.7.7/palette/color_difference/trait.Wcag21RelativeContrast.html
- https://docs.rs/notify/9.0.0-rc.5/notify/
- https://docs.rs/notify/9.0.0-rc.5/notify/fn.recommended_watcher.html
- https://docs.rs/notify-debouncer-full/latest/notify_debouncer_full/
- https://docs.rs/figment/0.10.19/figment/
- https://docs.rs/supports-color/latest/supports_color/
- https://docs.rs/ansi_colours/latest/ansi_colours/
- https://raw.githubusercontent.com/crossterm-rs/crossterm/master/src/style/types/colored.rs
- https://crates.io/api/v1/crates/tachyonfx/0.25.1/dependencies
- https://crates.io/api/v1/crates/tui-big-text/0.8.9/dependencies
- https://crates.io/api/v1/crates/ratatui/0.30.2/dependencies
- https://crates.io/api/v1/crates/figment/0.10.19/dependencies
- https://crates.io/api/v1/crates/notify-debouncer-full/0.8.0-rc.2/dependencies
- https://raw.githubusercontent.com/GNOME/vte/master/src/minifont.cc
- https://gitlab.gnome.org/GNOME/vte/-/issues/189
- https://no-color.org/
- https://raw.githubusercontent.com/aristocratos/btop/main/themes/dracula.theme
- https://clementtsang.github.io/bottom/nightly/configuration/config-file/styling/
- https://zellij.dev/documentation/themes
- https://raw.githubusercontent.com/zellij-org/zellij/main/zellij-utils/assets/themes/dracula.kdl
- https://wezterm.org/config/appearance.html
- https://raw.githubusercontent.com/alacritty/alacritty-theme/master/themes/dracula.toml
- https://raw.githubusercontent.com/rose-pine/alacritty/main/dist/rose-pine.toml
- https://raw.githubusercontent.com/rose-pine/palette/main/palette.json
- https://raw.githubusercontent.com/rose-pine/palette/main/README.md
- https://github.com/tinted-theming/home/blob/main/styling.md
- https://raw.githubusercontent.com/tinted-theming/base24/main/styling.md
- https://catppuccin.com/palette/
- https://www.nordtheme.com/docs/colors-and-palettes
- https://raw.githubusercontent.com/folke/tokyonight.nvim/main/extras/alacritty/tokyonight_night.toml
- https://raw.githubusercontent.com/morhetz/gruvbox/master/colors/gruvbox.vim
- https://draculatheme.com/contribute
- https://github.com/robb0wen/synthwave-vscode/blob/master/themes/synthwave-color-theme.json
- https://jfly.uni-koeln.de/color/
- local: /home/mattbeam/workspace/astral-watch/src/tui.rs, Cargo.toml
- local: cargo info (ratatui, crossterm, tachyonfx, tui-big-text, tui-widgets, palette, notify, notify-debouncer-full, figment, supports-color, ansi_colours, serde, colorsys, anstyle-query)
- local: fc-list :charset=…, fc-match, gsettings, dpkg -l, infocmp -x xterm-256color, tput colors, python3 WCAG/xterm-256 computation
