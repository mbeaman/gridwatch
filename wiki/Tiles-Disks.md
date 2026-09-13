# The disks tile

[![the disks tile](../docs/img/wiki/tile-disk.svg)](../docs/img/wiki/tile-disk.svg)

Per-device read and write rates, IOPS, service times and the drive's own
temperature, from one `/proc/diskstats` read per tick.

> **Press `3`.** This tile has a page of its own in the default layout. Until
> 2026-09-12 it shipped unplaced and no default install ever showed it — see
> [Configuring → where the disks tile lives](Configuring.md#where-the-disks-tile-lives)
> if you want it somewhere else.

## `BUSY` is not `%util`, and that is the point

Every other disk tool prints field 13 of `/proc/diskstats` as **utilisation**,
and on an NVMe drive that word is close to meaningless.

Field 13 is the share of wall time the queue was **non-empty**. It says nothing
about how *full* the queue was. A drive with one outstanding I/O and 1 022 free
slots reads **100 %** — a figure that would make you think the drive is the
bottleneck when it is barely awake.

Measured on the machine gridwatch is built against: a bounded direct read
sustaining **524 MB/s at 4 609 reads a second** reported **7.3 % util**, with an
average queue depth of 0.38. Seven percent, at half a gigabyte a second. The
number is not wrong; it is answering a question nobody asked.

So the column is called **`BUSY`**, and wherever the tile has the width for it,
**`Q`** is drawn immediately beside it:

| Column | What it is | `iostat` calls it |
|---|---|---|
| `BUSY` | Share of wall time the queue was non-empty | `%util` |
| `Q` | **Mean number of I/Os in flight** over the interval | `aqu-sz` |

`Q` is the one that tells you about saturation. Read them as a pair: `BUSY` high
and `Q` low is a drive that is *never idle but never loaded* — which is what a
desktop under light use looks like. `BUSY` high and `Q` in the tens is a drive
that is genuinely the bottleneck. The zoomed pane spells it out against the
queue's real depth:

```
busy 93% · q 39.3 of 1023 · r 1.30 ms · w 9.30 ms
```

Two things deliberately absent. **Field 12** — the instantaneous in-flight count
— is never published; it is a single sample of a number that swings wildly
between ticks. And **no figure here is derived from a lifetime counter**: the
service-time accumulators in diskstats are printed truncated to 32 bits and have
already wrapped on a machine that has been up a few days, so any "average since
boot" computed from them is an artifact. Everything on this tile comes from the
interval the sampler actually measured.

## What counts as a drive

A drive is a **sysfs fact**, not a name pattern: a `/sys/block` entry that has a
`device` link, is not `hidden`, and has a non-zero size. The `device` link exists
only for real hardware.

That rule matters more than it sounds. On a machine with snaps installed,
`/sys/block` holds dozens of loop devices — 52 of 55 entries on the reference
machine. Tools that filter by name prefix tend not to exclude `loop*`, and end
up summing several dozen squashfs mounts into "the disks". gridwatch's rule
excludes them because they have no `device` link, along with `dm-*`, `md*`,
`ram*` and `zram*`, which live under `devices/virtual/block/`.

If you legitimately want some of those back:

```toml
[sources.disk]
partitions = true              # also publish partitions, not just whole disks
extra = ["dm-*", "md0", "zram0"]   # opt specific refused devices back in
```

At most **16 devices** are published — drives first, then partitions, then
`extra` — and the [Sources](Tiles.md#sources) tile says how many were refused.
The cap exists because nine metrics across every line of a 64-line diskstats
would be roughly 22 MB of stored history against a 60 MB budget for the whole
process.

A device is classified the first time it is seen and forgotten when it leaves
diskstats, with no periodic re-walk — so **a USB drive you plug in appears on the
next tick**, within a second or two.

## The temperature comes from somewhere else

The `°C` column is not read by this tile. It is joined from the **sensors**
source, matching `disk.info{nvme0n1}.device` — which is `nvme0` — against the
hwmon chip's own device. Never by index: on the reference machine hwmon0, hwmon1
and hwmon2 are nvme1, nvme2 and nvme0 respectively, and any agreement between the
two numberings is a coincidence waiting to mislead you.

The cell reads `—` with the reason in the zoomed pane when the sensors source is
not running, no chip matches, or the drive is a SATA disk with no `drivetemp`
module loaded. A dash here means "not available", never "zero".

## Reading the rest

- **`R/S` and `W/S`** are completed reads and writes per second — `iostat`'s
  `r/s` and `w/s`.
- **The await columns** are the mean service time of a completed read or write.
  They are published *only on a tick where something completed*, so the tile
  shows a dash rather than a `0.0` that would mean "no data" and read as "instant".
- **There are no totals.** A "total" would change meaning the moment you set
  `extra`, and would disagree with a tile that is filtering. The tile sums what
  it is showing, and an alert rule on `disk.write_bps{*}` raises per device.

When the tile is too narrow for everything, columns drop in this order:
`Q`, `W/S`, `R/S`, `°C`, `BUSY`. The model name is elastic, so a wide tile fills
the space with the drive's actual name rather than stretching the numbers apart.

## Cost

One `/proc/diskstats` read per tick — about 4 KB — at 2 s, or 1 s when the tile
is visible, or 500 ms when it is focused. Classification happens once per device
name, not per tick. The whole pass is budgeted at under 1 ms of wall time and
0.2 % of one core, and `disk.scan_ms` on the [Sources](Tiles.md#sources) tile is
the evidence.

Unlike most tiles, this one never asks its source for more detail when you zoom
it: one file read serves every tier, including the zoomed pane.
