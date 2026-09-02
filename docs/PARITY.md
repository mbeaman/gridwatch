> **Status: opened in arc 1b (2026-08-31) with the htop section; the htop
> process-table rows ticked in arc 2a (2026-09-01); the nvtop section added in
> session 2b (2026-09-01).** astral-watch lands with arc 3 (§12.7). Every row is
> **in** (with the arc that ships it) or **out** (with the reason). A row is
> ticked by a test or by hand with a note — never by assertion. The parity arc
> (8) accepts by diffing against this file.

# Parity — what the emulated tools do, and what gridwatch does

Reference builds: **htop 3.4.1** (Ubuntu 3.4.1-5build2, sources at tag 3.4.1),
measured against torch's own `~/.config/htop/htoprc`, and **nvtop 3.2.0**
(`nvtop --snapshot`, upstream sources at tag 3.2.0) on driver 610.57.04.
Evidence for every claim is in `docs/research/htop-parity.md` and
`docs/research/nvtop-parity.md`.

## htop — header meters

| htop feature | gridwatch | Where |
|---|---|---|
| CPU meter, four segments (nice / user / kernel / virt), `detailed_cpu_time = 0` | **in — arc 1b** | `View::Segmented`, roles `AccentTertiary / Ok / Crit / Info`; the bar's `virt` segment is **steal + guest** and iowait counts as idle, exactly as htop's non-detailed mode does |
| `user -= guest`, `nice -= guest_nice`, `systemall = system+irq+softirq`, `idleall = idle+iowait`, `virt = guest+guest_nice`, `saturating_sub` deltas | **in — arc 1b** | `cpu::sampler::{Ticks, shares}`; pinned by `stat_breakdown_follows_htop`, `guest_is_subtracted_from_user_and_nice`, `deltas_never_underflow_on_a_counter_reset` against `fixtures/procfs/` |
| Per-CPU meters (`LeftCPUs4`/`RightCPUs4` on a 17–32 CPU box) | **in — arc 1b**, as two CCD blocks of paired SMT bars rather than htop's meter columns | `htop::view::ccd_block`; the grouping comes from sysfs `die_id`/`core_id` (`cpu.topology`), not from htop's cores-per-CCD guess |
| CPU temperature per CCD (`Tccd1`, `Tccd2` via libsensors) | **in — arc 1b**, read from k10temp **by label** | `cpu::sysfs::temp_inputs`; torch has no `temp2`, so indices are never assumed contiguous (`k10temp_is_resolved_by_label_not_by_index`) |
| Memory meter: used / shared / compressed / buffers / cache, text `used+shared+compressed / total` | **in — arc 1b** except `compressed` | `cached = Cached + SReclaimable − Shmem`, `used = MemTotal − (MemFree + Cached + SReclaimable + Buffers)`, pinned by `memory_formulas_match_htop`. **`compressed` is out:** zswap is disabled on torch and there is no `zram` device (research digest); it returns with the sensors arc if either appears |
| Swap meter: used / cache / frontswap, `usedSwap = SwapTotal − SwapFree − SwapCached` | **in — arc 1b** except `frontswap` (zswap, same reason) | `swap.used_b` / `swap.cached_b` |
| Tasks meter `{procs}, {uthreads} thr, {kthreads} kthr; {running} running` | **in — arc 2a, while the scan runs** | `tasks.kernel` is `PF_KTHREAD` counted by the pid-level scan (`Detail::Table`), so a tile whose demand reaches the scan prints htop's line — `procs = pids − kthr`, `uthreads = tasks − pids`, the same arithmetic as htop's `totalTasks − userlandThreads − kernelThreads` (`htop::format::tasks_htop`). A meters-only tile (no scan running) still says `641 pids, 2433 tasks; 3 running` rather than borrowing htop's words for different quantities. `running` is `/proc/stat`'s `procs_running`, uncapped (htop caps it at activeCPUs) |
| LoadAverage meter `%.2f/%.2f/%.2f` with OK/WARN/ERROR thresholds | **in — arc 1b** (values); the threshold colouring is **arc 3** with the alert rules | `htop::format::load` |
| Uptime meter `N days(!), HH:MM:SS` | **in — arc 1b**, shortened to `3d 06:01` on a grid tile | `htop::format::uptime`; the `(!)` past 100 days is out (cosmetic) |
| PSI meters (cpu / io / memory `some avg10`) | **in — arc 1b** | one row; `full`'s three-window (avg10/60/300) form is arc 8 |
| PSI **IRQ** meter | **out — the kernel does not expose it here** | torch has no `/proc/pressure/irq` (`CONFIG_IRQ_TIME_ACCOUNTING` is off) |
| Meter **modes** Bar / Text / Graph / LED | **out of arc 1** | the theme owns widget form (`[widgets]`, §7); per-meter modes would be a component option. BACKLOG: "htop meter breadth" (D39) |
| Configurable meter **sets** and the 13 header layouts | **out of arc 1** | gridwatch's layout is the grid itself; the tile picks its own two-column header at ≥ 76 cells. BACKLOG: "htop meter breadth" |
| DiskIO meter | **out of arc 1** | needs `/proc/diskstats` and a `disk` source — BACKLOG ("a `disk` component") |
| NetworkIO meter | **out of arc 1** | the `net` source and component are arc 7 |
| Clock / Date / DateTime / Hostname / SysArch / Battery / FileDescriptor / Systemd / SELinux / HugePage / ZFS / Zram meters | **out** | `clock` is its own component; the rest are either absent on torch or belong to `sensors` (arc 5) / a future `system` tile |
| GPU meter (htop 3.4) | **out — the data does not exist here** | htop reads DRM fdinfo; the proprietary NVIDIA driver exposes no `drm-*` lines (verified: zero fdinfo entries across own processes). gridwatch's GPU numbers come from NVML in arc 2 |

## htop — process table

| htop feature | gridwatch | Where |
|---|---|---|
| Top-N table, grid columns `PID RES SHR S CPU% MEM% TIME+ Command` | **in — arc 2a** (`table` tier, min 56×18; 10 rows at 250×70, 7 in a 4x2, 5–6 at 120×40 dense; zoom fills the body) | `htop::table`; pinned by `row_budget_at_the_real_grid_sizes` and `columns_drop_in_gridwatch_order` (gridwatch's drop order `SHR, TIME+, RES, PID, S, MEM%, CPU%` when `Command` would fall below `command_min` — htop scrolls instead, the grid cannot) |
| Irix-mode CPU% over `period = Δtotal / activeCPUs`, `percent_mem`, `TIME+`, `Row_printKBytes` regimes, `Row_printPercentage` (`100` in a 4-wide column, auto-width CPU% recomputed per scan cycle like `Row_resetFieldWidths`), state colours (`P` is `BLOCKED`, printed `B`) | **in — arc 2a** | `cpu::procs` (scan) and `htop::format::{kbytes,time_plus,percentage,state}` — `Row.c` 3.4.1 branch for branch; pinned against `fixtures/procfs/` (`cpu_percent_is_irix_mode_over_the_aggregate_period`) and by `kbytes_follows_row_print_kbytes` / `time_plus_follows_row_print_time` |
| Sort by any column (`F6`, `<`/`>`), invert (`I`), selection follows the PID across re-sorts | **in — arc 2a** (arc 2's read-only keys) | `Htop::on_key`; `keys_select_sort_and_invert` |
| `hide_kernel_threads` (default on), `PRI` prints `RT`, `NI` colours, thread rows in the thread role | **in — arc 2a** | `htop::table::cell`; thread rows only exist from `Detail::Columns` (arc 8) — the filter and the role are in place, unexercised |
| `USER` shadowed when not htop's own uid, magenta with elevated capabilities | **partial — arc 2a mutes root-owned rows** | the component may not call `getuid` (§4.6); the source publishing its uid is a BACKLOG item, capabilities need `status` (arc 8) |
| Row colours for the unit ladder (M cyan, G green, T+ red) and hour/day/year in `TIME+` | **in — arc 2a**, as theme roles `Info` / `Ok` / `Crit` | components never name a colour (§4.6) |
| Search `/`, filter `\\`, tags, tree, follow, horizontal scroll, `H`/`K` toggles | **arc 8** (`full`, zoom-only) | |
| `highlight_base_name`, `highlight_changes` (new rows green / tomb rows red) | **arc 8** | the options parse today (off by default, as in htop); the synth already has a PID that vanishes and reappears so the tomb rows have data to show |
| Full Main screen, I/O screen, tree view, search, filter, tags, follow, `K`/`H` | **arc 8** (`full`, zoom-only) | |
| Kill / renice / affinity / ioprio | **arc 8**, behind `readonly` and a confirm line (D35 #7) | |
| `hide_userland_threads` default | **deliberate deviation**: gridwatch defaults it **true** (htop: false) | a 6x3 tile must not be ten rows of one game's threads, and it keeps the scan pid-level (§8) |

## nvtop — header

| nvtop feature | gridwatch | Where |
|---|---|---|
| Line 1 `Device N[name]` + `PCIe GEN %u@%2ux RX: n unitB/s TX: n unitB/s` | **in — arc 2b** (`header` tier, min 56×8), with a recorded deviation in the rate text: gridwatch prints an integer with a 1024 step (`36 MiB/s`) where nvtop prints three significant digits with a 1000 threshold, to keep the 56-wide line inside its budget | `gpu::view::device_line`, `gpu::format::rate`; the PCIe half is built first and the name loses `NVIDIA `/`GeForce ` then is cut with its bracket (`Device 0 [RTX 50…]`) so the tier's signature survives at 56 wide. RX/TX come from the 32-bit byte-counter fields 197/198 diffed per second with the wrap handled (`counter_delta`) — never `pcie_throughput` (21 ms per call) |
| Line 2 `GPU %uMHz MEM %uMHz TEMP %3u°C FAN %3u%% POW %3u / %3u W`, `GPU` = max(graphics, SM clock), temperature green / yellow (slowdown − 5) / red (≥ slowdown) | **in — arc 2b** | `clocks_header_line`, `NvmlProbe::clock_gfx_mhz` = max(Graphics, SM), `temp_role` against `gpu.temp_slowdown_c` (93 °C on torch); the fan shown is the highest setpoint of the three |
| Line 3 `GPU[bar %]` `MEM[bar %]` — MEM is **VRAM occupancy** `used/total`, not `utilization_rates().memory` | **in — arc 2b**, with one recorded difference: nvtop's v2 call passes a wrong version stamp and falls back to `nvmlDeviceGetMemoryInfo` **v1**, whose `used` includes the driver's reserved VRAM (460 MiB on torch); gridwatch uses v2 (reserved excluded, what `nvidia-smi` prints) — idle, nvtop says `5%` where gridwatch and nvidia-smi say `1253 MiB` ≈ 3.8 % | `header_block`; `gpu.vram_used_b` (memory_info v2, reserved excluded) over `gpu.vram_total_b`. The memory-controller number is a separate `MEMCTL` gauge in the `gauges` tier so the two are never confused (digest §1: 43 % vs 5 % at the same instant); `header_keeps_pcie_at_minimum_width_and_mem_is_vram` |
| `ENC[bar]` `DEC[bar]`, auto-hidden after 30 s idle (`-E`); nvtop times each bar separately and starts both hidden | **in — arc 2b** with a recorded deviation: one shared timer, both bars shown for the first 30 s and hidden together | `Gpu::encdec_visible_at` on the store's clock (never `Instant`); `enc_dec_bars_hide_after_thirty_idle_seconds`. The freed row shows P-state and the throttle chip |
| Line 4 `-i` info bar `NSHC/L2CF/NEXC` | **out — NVIDIA never populates it** | nvtop prints N/A on this backend (digest §1). The GPU-Z spec column (`charts` tier at ≥ 100 wide: SMs, TMU/ROP, RT/tensor, L2, base/boost, memory rate, bandwidth, TDP, die, transistors, launch) covers what the bar was for, from the `const SPECS` row cross-checked against NVML |
| Charts: 10-minute ring buffer, one column per sample (so the window is `cols × interval`), fixed 0–100 % axis | **in — arc 2b** with a deliberate deviation: gridwatch always shows the last ten minutes resampled into one bucket per cell, whatever the width (`charts` tier, band 4–8 rows) | `store.resample` over `CHART_SPAN`, `View::Chart` drawn as connected braille segments by the theme's `chart_marker`; the window label follows the run's age up to `10m` |
| Plottable series: GPU util, GPU memory util (VRAM %), temperature, power draw %, GPU clock % | **in — arc 2b** as `util vram temp power clock`, keys `1`–`5`, option `series` | `gpu::view::series_points`; the `clock` ceiling is the once-published max clock carried into every bucket (`clock_series_has_points_despite_its_static_ceiling`) |
| Plottable: encoder/decoder rate, fan speed, memory clock | **out of arc 2** | ENC/DEC and fans are on the header; the six above cover the dashboard case. BACKLOG if wanted |
| Reverse plot direction (`-r`) | **in — arc 2b**, key `r` / option `reverse` | the legend's arrow flips with it |
| Refresh interval `-d` (default 1 s) | **in**, as the source's cadence: 500 ms visible / 250 ms focused / 1 s hidden (fast tier), 1 s slow tier | `[sources.gpu] refresh_ms`; the charts sample the store, not the poll |
| Colour bands on the bars, `-C` no colour | **in** through theme roles and gradients; `mono` is the no-colour mode | components never name a colour (§4.6) |
| `-f` Fahrenheit | **out** | Celsius only |
| Multi-GPU: one header block per device, plots per device, `DEV` column | **out of arc 2** — device 0 only (`[sources.gpu] device` selects another) | every key is labelled `{dev}` and the table auto-shows `DEV` with more than one device, so the seam is ready |
| Setup window `F2`, `F12` saves `interface.ini` | **out** | gridwatch is configured by `config.toml` / `layout.toml` (§9) |
| `-s` JSON snapshot | **out**; `gridwatch shot` renders a frame, `--record` a journal | |
| Extras nvtop 3.2 does not show on NVIDIA: fan **RPM**, P-state, throttle reasons (`PWRCAP`, `THERM`, `HW SLOW`, `BRAKE`), the 20 ms board-power trace (`samples(Power)`), memory-controller utilisation, max clocks, and the **effective load** chart series (`util × power / limit`, key `6` — the review found it is *not* an nvtop 3.2.0 metric; the research digest was wrong to attribute it to `extract_gpuinfo.c`) | **in — gridwatch additions** | `gpu.fan_rpm{dev:i}`, `gpu.pstate`, `gpu.throttle`, `gpu.power_trace`, `gpu.memctl_pct`, `gpu::view::effective_load`; recorded here so nobody files them as drift |
| Memory-junction temperature | **out — `NotSupported` on GeForce** | field 82 costs 1.2 ms to say so; never polled (digest §2) |

## nvtop — process table

| nvtop feature | gridwatch | Where |
|---|---|---|
| Columns and widths `PID(7) USER(≥4) DEV(3) TYPE(8) GPU(4) ENC(4) DEC(4) GPU MEM(14) CPU(6) HOST MEM(9) Command` | **in — arc 2b** (`procs` tier, min 56×18; grid default `PID DEV TYPE GPU GPU MEM CPU HOST MEM Command`, `DEV` auto-hidden with one device; `USER`, `ENC`, `DEC` via `columns` and `USER` in the zoomed `full`) | `gpu::table::Col::width`; `columns_drop_in_gridwatch_order` (gridwatch's drop order `ENC, DEC, HOST MEM, TYPE, USER, CPU` when `Command` would fall below `command_min = 12` — nvtop scrolls by four columns with `h`/`l`, the grid cannot); 10 rows at 250×70, 7 in a 4x2, 5–6 at 120×40 dense (`row_budget_at_the_real_grid_sizes`) |
| `TYPE` Graphic (yellow) / Compute (magenta); `Both G+C` exists in the UI but the NVIDIA backend never sets it — a PID in both lists is printed **twice** | **deliberate deviation**: gridwatch merges the two v3 lists by PID and shows **one** `Both G+C` row (three-coloured) | `sources::gpu::procs::merge`; `both_lists_merge_by_pid_and_own_pid_is_dropped`. The game and the terminal are in both lists on torch |
| `GPU`/`ENC`/`DEC` from `nvmlDeviceGetProcessUtilization` with `lastSeenTimestamp` carried forward; samples above 100 discarded; a process without a fresh sample reads 0 | **in — arc 2b**, plus `fresh = false` muting the zeros | `procs::overlay`, `Poller::procs`; `process_rows_merge_filter_and_carry_last_seen_forward` |
| `GPU MEM` `%6uMiB %3u%%` of `usedGpuMemory / total`, rounded | **in — arc 2b** | `gpu::format::gpu_mem` rounds as nvtop does (the review caught a truncation); `gpu_mem_is_nvtops_fourteen_cells` |
| `CPU` `100·(Δutime+Δstime)/Δwall` printed `%u%%`, `HOST MEM` RSS printed `%zuMiB`, `USER` via `getpwuid`, `Command` from `/proc/<pid>/cmdline` | **in — arc 2b** with a recorded deviation in the cell text — CPU keeps one decimal (`412.0%`, htop's figure) and HOST MEM uses a unit ladder (`12.5GiB`) — joined from the cpu source's `proc.table` (its CPU% is htop's Irix-mode figure, the same formula) with a per-PID last-known cache for `user`/`cmdline`; `—` when the cpu source is absent, `[pid]` when nothing was ever read | `gpu::table::Derived::rebuild`; `join_uses_the_scan_and_keeps_the_last_known_command`. The gpu tile's `demand(procs) = Detail::Table` raises the cpu scan on its own (§8), so the zoomed gpu tile has the columns with no htop tile visible (`shipped_placements_pick_the_expected_gpu_tiers`) |
| Sort by any of 11 criteria (`F6`), `+`/`−` direction | **in — arc 2b** as `<`/`>`/`F6` and `I` (arc 2's read-only key set); `+`/`−` and the setup window are arc 8 | `Gpu::on_key`; default `gpu_mem` descending; a context-only process (sm 0) sorts below an active one at equal memory (§8.1) |
| Selection with arrows, scrolling | **in — arc 2b**, keyed by PID across re-sorts | `keys_select_sort_invert_and_toggle_series` |
| `F9` "Send signal" menu (signals 1–31) | **arc 8**, behind `readonly` and a confirm line (D35 #7) | |
| `-P` hide the process list, `-p` no plots | **in** as tiers: a tile shorter than `procs` has no table; `view = "header"` pins a plotless tile | §4.6 `view` |
| Own process hidden | **in** — gridwatch is never a GPU client (P12) and filters its own pid from the lists anyway | `Poller::new(dev, own_pid)` |

## Verified by hand on torch, 2026-09-01 (arc 2b)

- `nvtop --snapshot` beside a 12 s `gridwatch run --record` on the idle box
  (release, 250×70 pty), the journal sample nearest the snapshot's instant:

  | field | nvtop / nvidia-smi | gridwatch (journal) |
  |---|---|---|
  | GPU util | 10 % | 13 % (the next fast tick: 20 %; the card idles in P8 and the number flickers per 500 ms) |
  | MEM | nvtop `5%` (v1, reserved included) / nvidia-smi `1253 MiB` | `1313275904` B = 1252.4 MiB ≈ 3.8 % |
  | temperature | 48 °C | 49 °C |
  | power | 50 W / 50.38 W | 49.0 W (`POWER_INSTANT`) |
  | GPU clock / MEM clock | 877 / 405 MHz | 870 / 405 MHz |
  | fan | 0 % | 0 % (three fans, `gpu.fan_pct{0:0..2}`) |
  | P-state | P8 | 8 |
  | PCIe RX / TX | `nvidia-smi dmon -s t`: 2–946 / 1–365 MB/s over four seconds | 516 / 243 MB/s mean over the recording, 798 MB/s peak — the byte-counter diff agrees in magnitude with the 20 ms-window API nvtop never shows |

- `cargo test -p gridwatch-sources --release --test gpu -- --ignored --nocapture`
  prints the static probe, the per-class NVML time (P11: **4.78 ms/s** with
  process rows) and every process row, and asserts gridwatch's own pid is not
  among them (P12); `nvidia-smi | grep -c gridwatch` = 0 while it runs.
- The `full` tier at 248×66 (zoomed) shows USER / CPU / HOST MEM / Command for
  the synth's game with no htop tile on the page, i.e. the gpu tile's
  `demand` raised the cpu scan on its own (`shipped_placements_pick_the_expected_gpu_tiers`).

## Verified by hand on torch, 2026-09-01 (arc 2a)

- The pid-level scan: 635 pids, 395 kernel threads, **5.4 ms mean / 6.3 ms
  worst** per pass in release (`cargo test -p gridwatch-sources --release
  --test procs -- --ignored`), against P15's 20 ms — the two-file pass costs
  a third of the digest's five-file estimate.
- `gridwatch run` beside `htop` (both default sort, CPU%): the same top rows
  in the same order, CPU% within one tick of each other, RES/SHR digits
  identical, TIME+ identical.

## Verified by hand on torch, 2026-08-31

- The live sampler's numbers were cross-checked against `free -b` (total
  identical; `used` within a sampling interval of `total − free − buff/cache`)
  and against the die map in `MACHINE.md` (CCD0 = cpu0–7 + 16–23, CCD1 =
  cpu8–15 + 24–31, SMT sibling of cpu *N* is cpu *N*+16).
- `cargo test -p gridwatch-sources --release --test cpu -- --ignored` prints the
  live scan and asserts every published value is in range.
