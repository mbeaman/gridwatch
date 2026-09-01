> **Status: opened in arc 1b (2026-08-31) with the htop section.** The nvtop
> section lands with arc 2, astral-watch with arc 3 (§12.7). Every row is
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
| Tasks meter `{procs}, {uthreads} thr, {kthreads} kthr; {running} running` | **out — arc 2** | all three of htop's counts need `PF_KTHREAD` from every `/proc/<pid>/stat` (its first field is `totalTasks − userlandThreads − kernelThreads`, its `thr` is *userland* threads), which is the pid-level scan `Detail::Table` gates. What arc 1b can count without that scan is **pid directories** and **all tasks**, so the tile says exactly that — `641 pids, 2433 tasks; 3 running` — rather than borrowing htop's words for different quantities. `running` is `/proc/stat`'s `procs_running`, uncapped (htop caps it at activeCPUs) |
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
| Top-N table, grid columns `PID RES SHR S CPU% MEM% TIME+ Command` | **arc 2** (`table` tier, min 56×18) | the view options (`sort`, `columns`, `table_rows`, `command_min`, `hide_kernel_threads`, `hide_userland_threads`, `highlight_*`) **already parse and validate in arc 1b** so a config written today keeps working |
| Irix-mode CPU%, `percent_mem`, `TIME+`, `Row_printKBytes` regimes, state colours | **arc 2** | formulas verified in the research digest; `fixtures/procfs/*/{1,2,<pid>}/stat|statm` are recorded and committed for those tests |
| Full Main screen, I/O screen, tree view, search, filter, tags, follow, `K`/`H` | **arc 8** (`full`, zoom-only) | |
| Kill / renice / affinity / ioprio | **arc 8**, behind `readonly` and a confirm line (D35 #7) | |
| `hide_userland_threads` default | **deliberate deviation**: gridwatch defaults it **true** (htop: false) | a 6x3 tile must not be ten rows of one game's threads, and it keeps the scan pid-level (§8) |

## Verified by hand on torch, 2026-08-31

- The live sampler's numbers were cross-checked against `free -b` (total
  identical; `used` within a sampling interval of `total − free − buff/cache`)
  and against the die map in `MACHINE.md` (CCD0 = cpu0–7 + 16–23, CCD1 =
  cpu8–15 + 24–31, SMT sibling of cpu *N* is cpu *N*+16).
- `cargo test -p gridwatch-sources --release --test cpu -- --ignored` prints the
  live scan and asserts every published value is in range.
