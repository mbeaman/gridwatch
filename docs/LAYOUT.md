# Layout

Where tiles go, and why they change shape when you resize the terminal.

Everything here is `layout.toml` — pages and placements, and nothing else. It
is **the only file gridwatch ever writes** (edit mode's `w`), which is why
behaviour lives in `config.toml` instead: an editor that rewrites your
hand-tuned refresh rates is an editor you stop using.

---

## The grid

A page is a fixed **12 columns × 6 rows** of units. Not pixels, not
percentages — units. A placement says where it starts and how many units it
takes:

```toml
schema = 1

[grid]
columns = 12
rows = 6
gap = 1
borders = "each"          # each | shared | none
cell_aspect = 0.5         # a terminal cell is about twice as tall as it is wide
min_unit_inner = { cols = 8, rows = 3 }

[[pages]]
name = "Overview"
hotkey = "1"
place = [
  { id = "cpu",  at = [0, 0], size = [6, 3], priority = 100 },
  { id = "gpu",  at = [6, 0], size = [6, 3], priority = 100 },
  { id = "pins", at = [0, 3], size = [4, 2], priority = 90 },
  { kind = "clock", at = [10, 5], size = [2, 1] },
]
```

`at` is `[column, row]` from the top left; `size` is `[columns, rows]`. A
placement names **either** an `id` — an instance you configured in
`config.toml` — **or** a `kind`, which places an anonymous instance with
default options. Never both.

Overlaps and out-of-bounds placements are refused at load with both names, so a
typo is an error you can read rather than a tile that vanished.

`cell_aspect` is why `6x3` looks square-ish and not like a letterbox: at 0.5 a
unit is twice as wide in cells as it is tall, which cancels the cell's own
shape.

---

## Three modes, chosen by your terminal's size

You do not choose the mode; the size does. The thresholds come from the grid
itself — `columns × (min_unit_inner.w + 2) + gaps` — not from a magic number,
so changing the grid moves them with it.

| mode | needs (default grid) | what changes |
|---|---|---|
| `configured` | **131 × 37** | the layout as written |
| `dense` | **109 × 27** | `gap = 0`, borders shared by a one-cell overlap, short titles, the tab bar hidden |
| `stack` | anything smaller | the page becomes one scrolling column, `priority` first |

Transitions back to a richer mode need **two** cells of headroom in both
directions, so a terminal sitting exactly on a threshold does not flicker
between modes while you drag it. `d` overrides the choice for the session.

`priority` only matters in stack mode: it is the order tiles appear in, highest
first. Everything else about a placement is ignored there.

Measured on a 250×70 terminal, a unit's *inner* size (what a component actually
draws into) works out as:

| footprint | 250×70 configured | 120×40 dense |
|---|---|---|
| 1x1 | 17×8 | 9×5 |
| 2x1 | 38×8 | 19×5 |
| 4x2 | 80×20 | 39×11 |
| 6x3 | 122×31 | 59×18 |

Every component's poorest tier fits **8×3**, so no placement is ever a useless
chip above stack mode.

---

## Tiers: one tile, several drawings

A component does not have a fixed appearance. It declares **cumulative tiers**,
poorest first, each with the inner size it needs, and the *real rect* picks the
richest one that fits. A 1x1 `gpu` is a utilisation badge; a 6x3 is nvtop's
header, charts and process table; zoomed it is the full sortable table.

```console
$ gridwatch component info gpu     # the tier ladder for one kind
$ gridwatch component list         # every kind (this is docs/COMPONENTS.md)
```

You can pin a tier rather than let the size choose:

```toml
{ id = "cpu", at = [0, 3], size = [12, 3], view = "meters" }
```

`view` names a tier. If the rect cannot fit it, the tile falls back to the
richest tier that does and shows a `view↓` chip so you can see it happened. An
unknown name warns at load and is ignored. `view` applies un-zoomed only —
zoom always gives the richest tier.

Some tiers are **zoom-only** (`z`): htop's and nvtop's full interactive tables
need 100×24 and a key bar, which is not something to hand a 6x3 tile in the
corner of a page.

---

## Editing without an editor

Press `e`. Move with `HJKL`, resize with `^h ^l ^j ^k`, cycle the footprint with
`s`, add with `a`, remove with `x`, undo with `u`, and save with `w`. The
gutters show a dotted unit grid so you can see what you are aiming at, and an
operation that would overlap or leave the page draws the rect it *tried* in red
and says why, rather than silently doing nothing.

`w` writes `layout.toml` through `toml_edit`, so your comments and formatting
survive. The full key list is [`KEYBINDINGS.md`](KEYBINDINGS.md).

Edit mode works in dense mode; in stack mode it tells you that edits apply but
are not drawn, because there is no grid on screen to draw them on.

---

## Pages

Each page is independent — its own placements and its own instances. `1`–`9`
jump by number, `[` and `]` step. **A page you are not looking at costs
nothing**: its sources drop to `Hidden` and it causes no redraws at all. That is
the cheapest way to keep a heavy tile around without paying for it.

---

## When something is not drawn

| you see | it means |
|---|---|
| a chip with a reason and a fix | a required capability is missing (`gridwatch doctor` lists them all) |
| `arrives in a later arc` | the kind is not in this build — check the feature flags |
| `this plugin sent no manifest` | an exec plugin did not answer in time ([`PLUGINS.md`](PLUGINS.md)) |
| `view↓` | the pinned `view` tier does not fit this rect |
| `STALE 12s` | the tile's source has not published for three of its own cadences |
| a starved chip (`▪ gpu`) | below stack mode's floor — the terminal is too small for anything |

`gridwatch config check` validates both files, lists the pages and placements it
parsed, and names anything it had to ignore.
