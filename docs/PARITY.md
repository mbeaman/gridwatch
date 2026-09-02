> **Status: opened in arc 1b (2026-08-31) with the htop section; the htop
> process-table rows ticked in arc 2a (2026-09-01); the nvtop section added in
> session 2b (2026-09-01); the astral-watch section added in session 3a
> (2026-09-02).** Every row is
> **in** (with the arc that ships it) or **out** (with the reason). A row is
> ticked by a test or by hand with a note — never by assertion. The parity arc
> (8) accepts by diffing against this file.

# Parity — what the emulated tools do, and what gridwatch does

Reference builds: **htop 3.4.1** (Ubuntu 3.4.1-5build2, sources at tag 3.4.1),
measured against torch's own `~/.config/htop/htoprc`, **nvtop 3.2.0**
(`nvtop --snapshot`, upstream sources at tag 3.2.0) on driver 610.57.04, and
**astral-watch `dce7eee`** (v0.7.0 + 3, `src/tui.rs` with the `tui` feature).
Evidence for every claim is in `docs/research/htop-parity.md`,
`docs/research/nvtop-parity.md` and `docs/research/astral-watch-and-sensors.md`.

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

## astral-watch — `tui.rs` (per-pin 12V-2x6)

| tui.rs feature (digest §1 "what tui.rs renders") | gridwatch | Where |
|---|---|---|
| Six per-pin bars: `█` fill by amps/10, `▔` session-peak cap, dim red `┄` at 9.2 A on empty cells, `8.2` value and `p1` labels; pin colour red > 9.2, yellow > 7.82, dark at 0 or stale, else green | **in — arc 3a** (`mini-bars` 20×4 without labels, `bars` 40×8 with values and labels) with **recorded deviations**: the bar *fill* is the `Power` gradient by height (the bands colour the values row), the limit line sits on the row that contains 9.2 A, a zero peak draws no cap | `pins::limit::PinBars` (`View::Custom`: the theme's `View::Bars` then the limit line through `Role::Crit` only — `limit_line_is_crit_role_on_empty_cells_only`); the amps bands are roles `Crit`/`Warn`/`TextMuted`/`Ok` (`pins::view::amps_role`); `AMPS_CEILING 10`, `OVERLOAD_A 9.2`, `IMBALANCE_ALARM_PIN_FRAC 0.85` verbatim from `keys/pins.rs` |
| `PIN_COLORS` Cyan/Green/Yellow/Magenta/Blue/LightRed as pin identity in the trend | **deliberate deviation**: six *roles* (`AccentPrimary, AccentSecondary, AccentTertiary, Info, Ok, Warn`) name the series in the legend; the trend's line colour is the `Power` gradient by height | components never name a colour (§4.6) |
| Totals `9.2 A ~111 W peak 620 W`, `pins 12.06–12.08 V · samples N` | **in — arc 3a** | `pins::view::totals_line`, `device_header` line 3 (`samples`) |
| Balance `Gauge`: ratio `(b−1)/(1.5−1)` clamped, `NORMAL` / `WARN` (> 1.33) / `ALARM` (> 1.5) / idle when total ≤ `min_load` | **in — arc 3a**; the `WARN` band stays the constant 1.33 while `ALARM` follows the configured ratio (tui.rs uses constants for both) | `pins::model::balance_class`, `balance_classes_follow_tui_rs`; the thresholds come from `pins.info` (astral-watch's own config) with the constants as fallback (D50 §4) |
| Watts `Sparkline` over `HISTORY = 300` samples | **in — arc 3a** (`trend` 60×14 and `full`; the badge's third row too) | `store.resample(pins.total_w)` over `history × interval` — the store's history, no second ring; `history` is the tile's one option |
| Braille trend `Chart` of six pins, y max = max(9.2, peak) | **in — arc 3a** (`full`, zoom-only) | `pins::view::pin_trend` through the arc-2b braille renderer |
| Alert log `List` + `Scrollbar`, `LOG_CAP 200`, newest at bottom, red/amber/green (repeats always yellow), grey status lines, wall-clock times | **in — arc 3a** (`trend`: 3+ lines; `full`: scrollable with `↑/↓ PgUp/PgDn`); **deviations**: `Repeated` keeps its severity's colour, no status lines (the `sources` tile has them), run-relative seconds | `pins::view::log` reads `store.alerts().events()` (the 500-event ring) filtered to the pins source; `Resolved` in `Ok` |
| Alarm row: red bg + white + BOLD + `SLOW_BLINK` `⚠ ALERT: OVERLOAD + DISCONNECT ⚠`; yellow, no blink, `IMBALANCE (ADVISORY)` for advisories only; `TELEMETRY LOST` shown as an alert | **in — arc 3a** inside the tile (`trend`, `full`) **and** as the cross-page banner; **deviations**: no `SLOW_BLINK` — the banner pulses reversed/plain on the heartbeat (D50 §7); the advisory-only state shows a `▲ N advisory` chip in the key bar and no banner; `TELEMETRY LOST` is `Info` — the `?` glyph, `STALE`, the `Degraded` status and the log, never the banner | `pins::view::alarm_row`; `Shell::banner_text`, `overlay::banner`; `the_alert_banner_is_on_every_page_and_acknowledges` |
| Device header line 1: badge + model + PCI + `PCIe Gen5×16` (yellow + `↓` below the card's max) + `i2c-N @ 0x2b` + `⏸ PAUSED` | **in — arc 3a** (`full`) — the model from astral-watch's card DB via `pins.info`, the PCIe link from the **gpu source's keys** (never sysfs), `PAUSED` while frozen; **deviation**: `↓` means below Gen5×16 (this card), the gpu source does not publish the link maximum yet (BACKLOG) | `pins::view::device_header`; `full_tier_reads_the_gpu_source_and_survives_without_it` |
| Device header line 2: `GPU 19%` bar, `PWR 107/600W` (green < 85 %, yellow < 97 %, red), `45°C` (green < 75, yellow < 85, red), `fan 30%` polled by an `nvidia-smi` subprocess every 1.5 s | **in — arc 3a** as text from the gpu source's keys with the same bands as roles; **out**: the `nvidia-smi` subprocess (gridwatch has the gpu source) and the bar glyphs | `device_header` line 2; `—` for every field when the gpu source is absent |
| Device header line 3: `connector 9.2 A · 111 W · balance 1.54× · pins 12.06–12.08 V`, `STALE ·` prefix when not live | **in — arc 3a** | `device_header` line 3; `STALE` when the last sample is older than 3 × the interval (`pins::view::stale`), also in the badge and totals |
| Compact 1-row header when height < 16 | **in**, as tiers: the header only exists in `full`; `trend` and below carry the totals line | §4.6 tiers instead of a height switch |
| Body ≥ 110 cols: bars 45 % over trend 55 % on the left 58 %, totals / balance / sparkline / log on the right 42 %; narrow: stacked | **in — arc 3a** (`full` follows the wide layout; the grid tiers are the stacked form) | `pins::view::full` |
| Keys: `q`/Ctrl-C quit, `space` pause sampling, `r` reset peaks, `+`/`-` rate, `1`–`5` zoom a panel (`0`/Esc back), `↑↓`/wheel scroll log, `Tab` card, `?` help | **in — arc 3a** with **deliberate renames**: `p` freezes the *display* (the source keeps sampling — gridwatch's `space` is the global pause and the banner must never be silenced, D50 §8); `r` peaks; `+`/`−` are faster/slower sampling by 100 ms as in tui.rs (500–5000 ms) through the first `Command::Source` in the product; `↑/↓ PgUp/PgDn Home` scroll the log. **Out**: `1`–`5` panel zoom (gridwatch's `z` zooms the tile), `Tab` cards (one card, arc 8), `?` (the shell's help) | `Pins::on_key`; `keys_freeze_reset_and_command_the_interval` |
| Multi-card tabs (`discover_cards`, one tab per PCI id) | **arc 8** | torch has one card; every key is unlabelled by device today |
| Interval clamp 100 ms–5 s | **deviation**: **500 ms–5 s** — P14 says ≥ 2 transactions/s never | `pins::clamp_interval` |
| Sampling continues while paused (`⏸` shows stale bars) | **in**, stronger: the source is `always_on` and never pauses; only the tile's picture freezes | §4.3 |
| Lifecycle: 3-of-5 confirm, 20 clean to resolve, repeat every 10 min, advisories at 240, TelemetryLost freezes the rest | **in — arc 3a**, astral-watch's own `Lifecycle` runs in the source with the policy from its own config file; gridwatch adds no debounce | `pins::bridge`; `an_overload_raises_and_resolves_through_the_bridge`, `telemetry_lost_is_info_and_freezes_the_rest` |
| `detect_bus` reasons (`NoBuses` / `PermissionDenied` / `NoTelemetry`), `redetect_card` after 10 misses, the deeply idle GPU's zeros as `TelemetryLost` | **in — arc 3a** | `pins::i2c::I2cBackend::{detect, explain, redetect}`, `Sampler::tick`; `losses_count_misses_and_redetect_at_ten` |
| Exporter (`/metrics`) as a telemetry source; its `alert_active` flags | **in — arc 3a** as the preferred backend in `auto`; the service's flags ride along as a `svc` chip and are **not** merged into the lifecycle (one debouncer, D50 §3) | `pins::exporter`, `pins::parse` (a 50-line parser, no `prometheus-parse`) |
| CSV tail | **arc 8** (D50 §5) | |
| `notify` transports (ntfy, webhook) | **out** — gridwatch is a viewer; the astral-watch service alerts (D51) | |

## audio — cava and the Winamp spectrum analyser (arc 5a, `docs/research/audio-capture-and-fft.md` §2–3)

| upstream feature | gridwatch | Where |
|---|---|---|
| cava: dual FFT — a long window for the bass bins, a short one above (8192 / 2048 at 48 kHz), Hann, `\|X\|·2/Σw` | **in — arc 5a** (`fft_bass` / `fft` under `[sources.audio]`; the split at 250 Hz) | `sources::audio::dsp::{Stage, Dsp}`; the §12.1 tests (1 kHz full-scale → its band ≥ 0.97 and RMS −3.01 dBFS; a bin-centred 52.7 Hz sine → one dominant band below 100 Hz; silence → zeros; the Hann identity) |
| cava: log-spaced bands between `lower_cutoff_freq` / `higher_cutoff_freq`, a band narrower than a bin interpolated | **in — arc 5a**: 64 bands `f_k = lo·(hi/lo)^(k/64)` between `lo_hz = 30` and `hi_hz = 16000`, max per band, linear interpolation between the two bins around a narrow band's centre | `dsp::Dsp::bands` |
| cava: `noise_reduction` (the integral smoother), `monstercat` neighbour filter, gravity fall `fall += 0.028`, `out = peak·(1 − fall²·g)` | **in — arc 5a** as the `cava` preset (`noise_reduction 0.77`, `monstercat 1.5`, `gravity 1.0`), frame-rate normalised to 30 fps | `components::audio::ballistics` (tests at 30 and 60 fps) |
| cava: bars as full-height columns with a gap (`bar_width`/`bar_spacing`), stereo mirrored (`reverse` the left channel) | **in — arc 5a**: ⌊(w+1)/3⌋ two-cell bars with a one-cell gap, the left channel reversed so the bass meets in the middle; `bars = N` gives thin bars | `components::audio::view::mirrored` |
| cava: `sensitivity` / autosens | **deviation**: a fixed dBFS floor (`floor_db = −65`) and `tilt_db_oct = 4` above 1 kHz instead of the auto-gain — a level meter must not lie about level | `dsp::DspConfig` |
| cava: the `source` option, PulseAudio/PipeWire backends | **in — arc 5a** over `pw-record` (+ `pw-dump` for the list): `s` opens a table of `Audio/Sink` nodes with state and default flag, `Enter` sends `Control::Domain(SetSink)`; `[sources.audio] sink = "auto" \| "<node.name>" \| <object.serial>` | `sources::audio::{sink, capture}`; never cpal/pipewire crates (D17) |
| cava: `framerate` | **in**: `[sources.audio] fps` (5–60) for the DSP cadence and the tile's `fps` option for the animation; `Redraw::No` once silent and settled | `RedrawPolicy::Animated { fps }`, D55 seam 5 |
| Winamp: 75 (or 19) bars, `falloff` 3/6/12/16/32 sixteenths per frame, instant rise | **in — arc 5a** as the `winamp` preset (`falloff 12`); the bar count from the width | `ballistics::Bars` (`Preset::Winamp`) |
| Winamp: peak caps that hold, then fall with an accelerating velocity (`v *= 1.1`) | **in — arc 5a**: caps hold 12 frames, then fall from 0.003/frame ×1.1 per frame; drawn as `▔` in the text role | `ballistics` (the peak-schedule test); the renderer's `Bars.peaks` |
| Winamp: the oscilloscope and the mode toggle | **in — arc 5a**: `m` cycles bars → scope → both; the scope is the latest 512 mono samples as a braille `View::Chart` (octants when the VTE marker is on), min/max downsampled per column | `components::audio::scope` |
| Winamp: the spectrum's colour ramp per row (fire), the peak colour | **deviation**: the theme's `Audio` gradient sampled per **bar height** (the renderer colours a column by its value), not per row; the cap in `Text` | `theme.gradients.audio`; D55 amendment 2 |
| VU / peak meters (stereo), dBFS text | **in — arc 5a**: the `vu` tier — `L`/`R` gauges over the RMS (−60..0 dBFS, 20 dB/s fall) with the source's 1.5 s peak hold | `ballistics::Vu`, `dsp::PeakHold` |
| LUFS (EBU R128 momentary / short-term) | **feature `audio-lufs`** (`ebur128`), keys declared; **not wired in 5a** — the `full` tier prints `—` | `keys::audio::{LUFS_M, LUFS_S}` |
| Reacts to Firefox / game audio within ~30 ms | **Matt's row** — an agent does not start players; `torch-audio.jsonl` is the silence path | `docs/PERFORMANCE.md` P16 |

## sensors — `sensors(1)` / `lm-sensors` and nvtop's thermals (arc 5b)

| upstream behaviour | gridwatch | Where |
|---|---|---|
| `sensors` walks `/sys/class/hwmon`, reads `<stem>_input` with `<stem>_label`, and prints per chip | **in — arc 5b**: the same walk for `temp*`, `fan*`, `in*`, `power*`, labels from `_label` else the stem | `sources::sensors::hwmon::walk` |
| `sensors` names chips `k10temp-pci-00c3` (driver + bus) | **deviation**: chips are keyed by hwmon `name` with a `#2` suffix for a duplicate (`nvme`, `nvme#2`, `nvme#3`) — hwmon numbering is not stable across boots and the bus id is not in the store's label vocabulary | `hwmon::walk`; D55 5b amendment |
| `sensors` prints `(high = +81.8°C, crit = +84.8°C)` from `_max`/`_crit` | **in — arc 5b**, once per generation as `sensor.max_c` / `sensor.crit_c`; the nvme `65261850` m°C sentinel is dropped | `hwmon::threshold`; the fixture test |
| `sensors` shows every reading always | **deviation**: the tile sorts by the **margin to `max`** (hottest = closest to its own limit), so a 59 °C `Tctl` with no max never hides an 80 °C NVMe at 82 °C max; `o` switches to a by-chip order | `components::sensors::Model::refresh` |
| `sensors` unit scaling (m°C, mV, µW, RPM as-is) | **in — arc 5b** | `hwmon::Kind::divisor` |
| `sensors -j` / watch loops re-reading every 1 s | **in**: `[sources.sensors] refresh_ms` clamped 500–10000 (NVMe `temp*_input` is a SMART log page, spd5118 an SMBus read, k10temp an SMN read) | `sources::sensors` |
| RAPL package power (`turbostat`, `powerstat`) | **in — arc 5b** when `energy_uj` is readable: `Δenergy mod max_energy_range_uj / Δt`; on torch it is 0400 root-only, so the tile and `doctor` print the udev rule instead | `sources::sensors::rapl`; `RaplState` |
| nvtop's GPU temp / fan / power | **not duplicated**: the `full` tier reads the gpu source's keys (`gpu.temp_c`, `gpu.fan_pct`, `gpu.power_w`) and says `no gpu source` without them | `components::sensors::view::gpu_line` |
| PSI (`/proc/pressure/*`) | **in — arc 5b** in the `full` tier from the cpu source's `psi.*` keys (the cpu source owns them) | `view::psi_line` |
| `sensor.temp_c{k10temp:*}` published by the cpu source (arcs 1b–5a) | **handover — arc 5b**: with the `sensors` feature the cpu source starts with `k10temp = false` and stops publishing them; the sensors source publishes the same key with the same labels, so htop's `Tccd` column is unchanged | `sources::cpu::k10temp_default`, §16 |

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
