> **Status: opened in arc 1b (2026-08-31).** Which glyphs the shipped themes
> rely on, whether this machine can draw them, and what each built-in theme is
> for. Theme *mechanics* (roles, gradients, the loader ladder, `class`) are
> §7 of `ARCHITECTURE.md`; this file is the practical companion.

# Themes on torch

## The glyph check

gridwatch's `unicode` glyph tier is the default because **no Nerd Font is
installed** (`MACHINE.md`). Everything the tier uses must therefore come out of
DejaVu-class fonts or VTE's own box-drawing. The 30-second check — run it in the
terminal you actually use:

```sh
printf 'rounded ╭─╮ │ ╰─╯   double ╔═╗ ║ ╚═╝   thick ┏━┓ ┃ ┗━┛\n'
printf 'eighths ▁▂▃▄▅▆▇█  left ▏▎▍▌▋▊▉█  shade ░▒▓\n'
printf 'braille ⠁⠂⠄⡀⢀⠈⠐⠠⣿⠿  octant 𜺠𜺣🯦🯧▘▝▖▗▚▞\n'
printf 'katakana ﾊﾐﾋｰｳｼﾅﾓﾆｻ (matrix, arc 4)\n'
```

Every glyph must render as **one cell wide**, with no boxes, no blanks and no
column drift on the lines that follow.

| glyph class | codepoints | used by | fontconfig resolves to | drawn correctly in Ptyxis |
|---|---|---|---|---|
| rounded corners | U+256D–U+2570 | `modern` borders | DejaVu Sans | **pending Matt** |
| double / thick borders | U+2550–U+256C, U+250F–U+251B | `retrowave` (`double`, focused `thick`) | DejaVu Sans | **pending Matt** |
| eighth blocks (vertical) | U+2581–U+2588 | every bar, sparkline and core block | DejaVu Sans | **pending Matt** |
| eighth blocks (horizontal) | U+258F–U+2588 | `GlyphSet::partial_h` — **no caller yet**: the gauge renderer draws whole cells today, so this row is checked ahead of the first partial-width gauge | DejaVu Sans | **pending Matt** |
| shade | U+2591–U+2593 | empty-bar fill | DejaVu Sans | **pending Matt** |
| braille | U+2800–U+28FF | the gpu charts (arc 2b; `[glyphs] chart_marker = "braille"` is the default, `"block"` and `"dot"` are the alternatives, the `ascii` tier always gets `*`) | DejaVu Sans | **pending Matt** |
| octants | U+1CD00–U+1CDE5 | `chart_marker = "octant_if_vte"` | Noto Sans — **VTE 0.84 draws these itself**, which is why the setting is conditional | **pending Matt** |
| half-width katakana | U+FF66–U+FF9D | the `matrix` rain (arc 4) | Noto Sans CJK JP | **pending Matt** |

Fontconfig resolution above was probed on 2026-08-31 with
`fc-match -f '%{family}\n' ':charset=<cp>'`; the terminal's own font is
`DejaVu Sans Mono` (`fc-match monospace`). Resolution is not the same claim as
"renders as one cell in Ptyxis" — that is the eyeball half of the check, and it
is the one row of this arc a machine cannot tick for itself.

## The built-ins

| theme | class | what it is for | notable choices |
|---|---|---|---|
| `retrowave` | quiet | the default; Synthwave '84 palette with the computed contrast fixes | `double` borders (`thick` when focused), gradient upper-case titles, eight gradients, `gauge = "line"` |
| `modern` | quiet | Catppuccin Mocha; the reference theme every cell snapshot is taken in (§12.2) | `rounded` borders, plain titles |
| `mono` | quiet | no colour at all — the real theme `NO_COLOR` loads, because crossterm 0.29 emits no colour SGR in that mode | severity survives as glyph + `BOLD`/`REVERSED` |

`terminal` and `phosphor-green` arrive with the theme loader v2 in arc 3;
`phosphor-amber` and the showcase-class `matrix` in arc 4.

## What a theme may decide (and what it may not)

A theme owns **paint and form**: the 19 role colours, the eight gradients, the
glyph tier, borders, title style, and the widget variants in `[widgets]`
(`gauge`, `bars`, `sparkline`, `table_header`, `big_number`). A component owns
**content**: which roles and which gradient a value is drawn with. That split is
why the htop tile looks like htop in every theme without naming a single colour
— its CPU meter asks for `Ok`, `Crit`, `AccentTertiary` and `Info`, and each
theme answers in its own palette.

Role swatches for all three built-ins are pinned by
`crates/ui/tests/ui.rs::role_swatches_pin_the_palettes`, so a palette edit shows
up as a reviewable diff rather than a surprise on screen.
