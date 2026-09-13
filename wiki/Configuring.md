# Configuring

Two files, and they are deliberately separate:

| File | Holds | Written by |
|---|---|---|
| `~/.config/gridwatch/config.toml` | **Behaviour** — theme, frame rate, which components exist, source options, alert rules | You |
| `~/.config/gridwatch/layout.toml` | **Pages and placements** — where tiles sit on the grid | You, or edit mode (`e`) |

The split exists so that rearranging tiles in the app can never rewrite the file
you hand-edited. Edit mode writes `layout.toml` and nothing else, preserving your
comments.

Write yourself a starting pair:

```sh
mkdir -p ~/.config/gridwatch
gridwatch config default > /tmp/both.toml   # prints both files, clearly marked
```

Both files are reloaded about once a second when their modification time
changes, so you can keep the dashboard open while you edit.

Check them before wondering why something did not take:

```sh
gridwatch config check
```

It parses both files, validates every source option's **type** as well as its
name, lists your alert rules, and reports the theme's colour contrast. A
mistyped key or a `refresh_ms = "1000"` where a number belongs exits non-zero
with a sentence naming the key, the bad value and the fix — rather than silently
falling back to a default, which is the failure mode this command exists to end.

## A component is declared, then placed

Declaring a component in `config.toml` gives it an **id** and options:

```toml
[[components]]
id = "cpu"
kind = "htop"
```

Placing it in `layout.toml` puts that id somewhere on a 12x6 grid:

```toml
{ id = "cpu", at = [0, 0], size = [6, 3], priority = 100 }
```

`at` is `[column, row]` from the top left, `size` is `[columns, rows]` in grid
units. `priority` decides who survives when the terminal is too small to draw
everything — higher wins.

Tiles that need no configuration — `clock`, `sources`, `alerts` — can skip the
declaration and be placed by kind directly:

```toml
{ kind = "clock", at = [10, 5], size = [2, 1] }
```

## Where the disks tile lives

The [disks tile](Tiles-Disks.md) has **page `3`** to itself, and that is the
whole of the default arrangement for it. It is not on the Overview because the
Overview is full: every slot it could take belongs to a tile that a test or the
performance measurement protocol depends on, so rather than displace one it got
a page. A page costs nothing — the `disk` source runs either way, so the only
effect is a cadence rising from 2 s to 1 s while you are looking at page 3.

*(Before 2026-09-12 it shipped with no placement at all, and no default install
ever showed it.)*

If you want it somewhere else, this is what the default does, and it is the
same pattern for any tile. In `config.toml`:

```toml
[[components]]
id = "drives"
kind = "disk"
```

and to `layout.toml`:

```toml
[[pages]]
name = "Disks"
hotkey = "3"
place = [
  { id = "drives", at = [0, 0], size = [12, 6] },
]
```

`[` and `]` cycle pages, or press the `hotkey`. To move it onto the Overview
instead, `e` enters edit mode, where you move and resize tiles with the keyboard
and `w` saves — and edit mode only ever writes `layout.toml`, so your
`config.toml` comments are safe.

For a smaller footprint, `4x2` is the recommended size: that gets you the table
with rates, `BUSY`, `Q` and a temperature per drive.

## Source options

Each source is configured once, under its own section — never per tile, because
a source is a singleton and two tiles asking for different cadences would be
ambiguous:

```toml
[sources.cpu]
refresh_ms = 1500

[sources.disk]
partitions = true
extra = ["dm-*", "zram0"]
```

Options that only affect *appearance* belong on the placement instead, as
`view = "..."` or per-component keys. The full option list per source is in
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md) §8.

## How long history is kept

```toml
[store]
history = "10m"    # 1m to 1h
```

Charts draw what the store actually holds, so a tile asking for a ten-minute
window on a one-minute history shows the minute that exists rather than a
stretched line. Raising this costs memory roughly in proportion. Changing it
needs a restart, and gridwatch tells you so rather than pretending it took.

## Alert rules

Any metric can raise an alert, and a threshold can be **another metric** rather
than a constant:

```toml
[[rules]]
name = "drive over its own limit"
key = "sensor.temp_c{nvme*:Composite}"
op = ">="
value = "sensor.crit_c"     # a metric, not a number
for_s = 10
severity = "crit"
```

Two things are doing work there. The **label glob** makes it one alert per drive
rather than one alert about all of them. And `value` is **another metric read at
the same label**, so each drive is compared against its own critical threshold
and nothing is hard-coded — which is the difference between a rule that survives
new hardware and one that does not.

`op` also accepts `absent`, for a key that stopped arriving. Those need a
`for_s`, since a rule without one would be true from the first frame before any
data exists.

`config check` lists every rule it parsed, which is the fastest way to discover
that a glob matched nothing. `config default` ships several worked examples,
commented out.

---

See also: **[Themes](Themes.md)** · **[Keys](Keys.md)** ·
**[Troubleshooting](Troubleshooting.md)**
