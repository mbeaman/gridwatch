> **Status: opened in arc 1b (2026-08-31) with the htop section; the htop
> process-table rows ticked in arc 2a (2026-09-01).** The nvtop section lands
> with session 2b, astral-watch with arc 3 (§12.7). Every row is
> **in** (with the arc that ships it) or **out** (with the reason). A row is
> ticked by a test or by hand with a note — never by assertion. The parity arc
> (8) accepts by diffing against this file.

# Parity — what the emulated tools do, and what gridwatch does

Reference builds: **htop 3.4.1** (Ubuntu 3.4.1-5build2, sources at tag 3.4.1),
measured against torch's own `~/.config/htop/htoprc`. Evidence for every claim
is in `docs/research/htop-parity.md`.

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
