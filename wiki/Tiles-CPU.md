# The CPU tile

[![the CPU tile](../docs/img/wiki/tile-htop.svg)](../docs/img/wiki/tile-htop.svg)

htop, reimplemented rather than approximated. The formulas below are htop
3.4.1's own, taken from its source, because a CPU meter that disagrees with htop
by two percent is worse than no CPU meter.

## The meters

**`CPU`** is Irix-mode percentage — the same convention htop uses by default,
where a fully loaded 16-core machine reads 1600 %, not 100 %, and the bar is
scaled accordingly. The bar is segmented: nice, user, kernel and virtualised
time each get their own colour through the theme's roles, so you can see *what
kind* of busy it is at a glance rather than just how much.

Two details that trip up most reimplementations, both handled here:

- **Guest time is subtracted** from user time. The kernel counts guest cycles in
  both `user` and `guest`, so adding them naively double-counts a VM's load.
- **`iowait` counts as idle.** It is not time the CPU spent working.

**`MEM`** uses htop's cached formula — `cached = Cached + SReclaimable − Shmem`
— which is not what `free` prints and is the more useful number: it excludes
shared memory that is not reclaimable under pressure.

**`SWP`** includes swap-cached pages as a distinct band, which htop added and
most clones did not.

## The core blocks

Cores are not listed in a flat row of numbers. They are grouped into the **real
CCD blocks** your processor has, read from sysfs `die_id`, with SMT siblings
paired — so on a dual-CCD Ryzen you get two blocks of eight, each with its own
frequency and its own `Tccd` temperature.

This matters on a chip where one die carries the stacked cache and the other
clocks higher: the two dies behave differently under load, and a flat list of
sixteen bars hides exactly that.

The temperature comes from `k10temp` until the **sensors** source is running, at
which point sensors takes the key over. You do not need to configure the handover.

## `PSI` — the row worth learning

```
PSI cpu 1.37 · mem 0.19 · io 1.09
```

**Pressure Stall Information**, from `/proc/pressure/*`, and the most useful
three numbers on the tile once you know what they are.

Each is a percentage of the last ten seconds during which **at least one task
was stalled** waiting for that resource. Not "how busy is the CPU" — how much
work was *blocked*. That distinction is the whole value:

- `cpu 1.37` — 1.37 % of the last ten seconds, something runnable was waiting
  for a core. Near zero on an idle machine, climbing well before the CPU meter
  pins.
- `mem 0.19` — time spent stalled on memory reclaim. A machine that looks fine
  on the MEM bar but is thrashing shows it here first.
- `io 1.09` — time stalled on storage. Pair this with the
  [disks tile](Tiles-Disks.md)'s `Q` column and you can tell a slow drive from a
  busy one.

PSI rises before the conventional meters do, which is why it is worth a glance
even when everything looks green.

## Tasks, load and uptime

```
210, 1792 thr, 431 kthr; 3 running · load 4.51 4.20 3.52
```

`210` processes, `1792` userland threads, `431` kernel threads, `3` currently
runnable. This is htop's own wording, and the kernel-thread count appears only
while the process scan is running — which is to say, only when a tile is
actually showing a process table.

## The process table

The grid table shows the top N (10 by default, never fewer than 5) with a
decluttered column set: `PID RES SHR S CPU% MEM% TIME+ Command`. `USER`, `VIRT`,
`PRI` and `NI` are off on the grid for space and available through the `columns`
option and in the zoomed table.

**One deliberate deviation from htop's defaults:** userland threads are hidden.
htop shows them by default; gridwatch does not, because ten rows of one game's
render threads is not a useful grid tile, and because hiding them keeps the
`/proc` scan at the process level instead of walking every task directory. Press
`H` in the zoomed tier to turn them on — that both flips the option and tells
the source to start the deeper walk.

`Command` is always the last column, whatever order you write `columns` in,
because a variable-width free-text column anywhere else pushes everything after
it off the edge.

## Zoomed: the full htop

Press `z` and the tile becomes the interactive table — this tier is zoom-only,
so it is not in the screenshot above:

- **search** `/`, **filter** `\`, **tree view**, **tags**, **follow**
- every column, with horizontal scrolling
- sort by any column
- `K` and `H` to toggle kernel and userland threads
- the F-key bar

And the four actions, each behind a confirm line: **kill** (with a signal menu),
**renice**, **CPU affinity** and **I/O priority**. Set `readonly = true` in
`config.toml` and they are refused outright.

## Cost

The process scan reads `stat`, `statm`, the directory owner and — only on first
sight of a process — `cmdline`. That is about **5.4 ms for 635 processes**, on
its own schedule of 3 s while visible, 1.5 s while focused, which is slower than
the meters update.

The expensive files htop reads — `io`, `smaps_rollup`, `cgroup`, `oom_score`,
`exe`, `cwd` — and the per-thread walk are only touched when a tier actually
displays those columns.

## One thing that reads as a dash

htop 3.4 added a GPU meter and a `GPU%` process column. Both read DRM fdinfo,
which NVIDIA's proprietary driver does not expose — verified: no `drm-*` lines
appear for any GPU client. htop prints `0.0` or `N/A`; gridwatch prints a dash,
because a zero that means "cannot know" is a lie. Use the
[GPU tile](Tiles-GPU.md), which reads NVML instead.
