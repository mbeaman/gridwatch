# The GPU tile

[![the GPU tile](../docs/img/wiki/tile-gpu.svg)](../docs/img/wiki/tile-gpu.svg)

nvtop 3.2.0's header and charts, over NVML. **NVIDIA only** — there is no AMD or
Intel path, and the tile says so rather than showing an empty frame.

## The header

```
Device 0 [RTX 5090]    PCIe GEN 5@16x RX: 36 MiB/s TX: 11 MiB/s
GPU 2769MHz MEM 14001MHz TEMP 44°C FAN 29% POW 386/600W
```

This is nvtop's layout, including its auto-hiding: the encoder and decoder
figures only appear when something is using them, because a permanent `ENC 0%`
is noise.

**PCIe throughput** is computed from the card's byte counters, diffed between
ticks — not from NVML's `pcie_throughput` call, which samples over an interval
you do not control and is markedly more expensive.

**Fan** is read as both a percentage and RPM every five seconds. nvtop reads the
percentage only; RPM is the more honest number when a card has a zero-RPM idle
mode, because 0 % and 0 RPM mean different things.

## Two memory numbers that are not the same

| | |
|---|---|
| **`VRAM`** | How much video memory is allocated |
| **`MEMCTL`** | How busy the **memory controller** is |

They are independent. A card can be at 95 % VRAM and 3 % MEMCTL (a big model
sitting idle in memory) or 20 % VRAM and 90 % MEMCTL (a small working set being
hammered). Most tools show only the first. If a workload is slower than its
utilisation figure suggests, MEMCTL is usually where the answer is.

VRAM is read through NVML's v2 memory call, which **excludes driver-reserved
memory** — so the total will be slightly under the number on the box, and the
used figure is the one you can actually do something about.

## The charts

Ten minutes of history, drawn as braille lines, with quarter gridlines so the
rows between them read as an axis. Six series, selected with keys `1`–`6`:

1. **utilisation**
2. **VRAM %**
3. **temperature**
4. **power %**
5. **clock %**
6. **effective load** — `util × power / limit`

The sixth is not an nvtop metric. It exists because utilisation alone is
misleading on a modern card: a GPU can sit at 99 % utilisation while drawing 40 %
of its power limit, which means it is issuing work but stalling on something.
Effective load multiplies the two, so a genuinely saturated card reads high and a
stalled one does not.

`r` reverses the chart direction. The **GPU-Z-style spec strip** — driver
version, VBIOS, core count, bus width, power limits, the thermal slowdown
threshold — appears under the chart when the tile is wide enough, as a wrapped
strip rather than a column, so the chart keeps the full width.

## The process table

The GPU process list joined with the CPU scan, so each row carries both sides:
`PID DEV TYPE GPU GPU MEM CPU HOST MEM Command`. The `USER`, `CPU` and
`HOST MEM` columns come from the **cpu** source — if it is not running they read
as dashes, and the tile still works.

Per-process GPU accounting is **only polled while a tile is actually showing
it**. Zoom out of the process tier and those NVML calls stop.

## What it costs, and why that is designed

The tile is polled on two schedules: a fast tier at 500 ms (250 ms while
focused) for utilisation, temperature, power, clocks and throttle state, and a
slow tier at 1 s for memory, encoder/decoder and PCIe counters. The board-power
trace samples at 20 ms, and only while a GPU tile is visible.

Total NVML time is about **4.3 ms per second** with process rows on. This is
measured and gated, not estimated, because NVML calls are not free and the
driver is shared with whatever else is using the card.

**gridwatch is never itself a GPU client.** It does not open a rendering context
or allocate video memory, so it does not appear in its own process table and
does not contend with what you are actually running.

## When the card is not there

| What happened | What you get |
|---|---|
| `libnvidia-ml.so.1` not found | The tile names the library and the package that carries it, and falls back to parsing `nvidia-smi` at 1–2 s — the header still works, process rows do not |
| Driver/library version mismatch | "driver/library mismatch — reboot", and no retry loop |
| A field the card does not support | Never polled again; the cell is a dash |
| The card disappears (`GpuLost`) | Re-initialise with backoff |
