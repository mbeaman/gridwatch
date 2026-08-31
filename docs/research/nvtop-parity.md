<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Reproducing nvtop 3.2 for the RTX 5090 via NVML (nvml-wrapper 0.12.1), plus GPU-Z-style static specs from the gpuwatch reference — API mapping, verified behaviour on driver 610.57.04, polling costs, size-class plan, nvidia-smi fallback

## 1. What nvtop 3.2.0 actually shows (inventory)

Verified from the installed binary (`nvtop --version` = 3.2.0, `nvtop --help`, `nvtop --snapshot`) and the upstream sources (`src/interface.c`, `src/extract_gpuinfo_nvidia.c`, `src/extract_gpuinfo.c`, `src/interface_setup_win.c`, `manpage/nvtop.in`).

**Per-GPU header (3 lines, +1 optional):**
- Line 1: `Device N[<name>]` then `PCIe GEN %u@%2ux RX: <n> <unit>B/s TX: <n> <unit>B/s` (units scale by 1024: B/s, KiB/s, MiB/s, GiB/s). Max gen/width are fetched once (static) but nvtop only prints current gen@width.
- Line 2: `GPU %uMHz` (max of graphics/SM clock), `MEM %uMHz`, `TEMP %3u°C` (green below slowdown-5, yellow near slowdown, red at/above), `FAN %3u%%`, `POW %3u / %3u W` (power_draw / enforced power limit).
- Line 3: `GPU[bar %]`, `MEM[bar %]`, `ENC[bar]`, `DEC[bar]` — ENC/DEC auto-hide after 30 s idle (`-E`, `encode_decode_hiding_timer`). **Important nuance:** nvtop's MEM bar is VRAM occupancy (`used/total`), not NVML's memory-controller utilisation. Verified: `nvtop --snapshot` printed `mem_util 43%` while `nvmlDeviceGetUtilizationRates().memory` was 5% at the same moment.
- Line 4 (`-i` / "Display extra GPU info bar"): `NSHC/L2CF/NEXC` — the NVIDIA backend never populates these, so it prints N/A on NVIDIA. Not worth reproducing via nvtop's route; get cores/L2 from NVML + a spec table instead (see §4).
- Nothing about PCIe max gen, throttle reasons, P-state, memory temperature or fan RPM is shown by nvtop 3.2 on NVIDIA (it calls only `nvmlDeviceGetFanSpeed` v1 and `nvmlDeviceGetTemperature(GPU)`).

**Charts:** ring buffer allocated as `interface_alloc_ring_buffer(devices, 4, 10*60*1000, ...)` = 10 minutes of samples at the update interval (600 samples at 1 s); one column per sample, so the visible window is the plot width. Y axis fixed 0–100 % with 25/50/75/100 ticks; x labels `update_interval*cols/4/column_divisor/1000` s. Plottable metrics (Setup → Chart, per GPU or all GPUs): GPU utilization rate, GPU memory utilization rate, encoder/decoder rate, temperature, power draw rate, fan speed, GPU clock rate, memory clock rate, and **effective load rate** = `gpu_util_rate * (power_draw / power_draw_max)` capped at 100 (computed generically in `extract_gpuinfo.c`). "Reverse plot direction" (`-r`) puts recent data on the left. Default refresh 1 s (`-d` in tenths of a second).

**Process table columns (widths):** PID(7) USER(4) DEV(3) TYPE(8) GPU(4) ENC(4) DEC(4) GPU MEM(14, "12579MiB 38%") CPU(6) HOST MEM(9) Command. TYPE is Graphic (yellow) / Compute (magenta); "Both G+C" exists in the UI but the NVIDIA backend never sets it (a PID in both lists is just listed by the graphics pass first). GPU%/ENC/DEC come from `nvmlDeviceGetProcessUtilization` (smUtil/encUtil/decUtil capped at 100; memUtil is fetched but unused). GPU MEM from `usedGpuMemory` with `% = used/total`. CPU% = `100*(Δutime+Δstime)/Δwall` from `/proc/<pid>/stat`; HOST MEM = RSS pages × page size; USER via `getpwuid(st_uid of /proc/<pid>)`; Command from `/proc/<pid>/cmdline`. Sort (F6) by any of 11 criteria; `+`/`-` ascending/descending; F9 opens a "Send signal" menu with signals 1–31; F2 setup (General / Devices / Chart / Processes / GPU Select), F12 saves `~/.config/nvtop/interface.ini`; F10/q/Esc quit; arrows select/scroll. `-P` hides the process list, `-p` disables plots, `-s` prints a JSON snapshot (60 ms wall on this box), `-f` Fahrenheit, `-C` no colour. Multi-GPU: one header block per GPU, plots mapped per device, DEV column disambiguates processes.

## 2. Metric → nvml-wrapper 0.12.1 mapping (verified against the local crate source and live on the RTX 5090 / driver 610.57.04)

Crate facts (verified in `~/.cargo/registry/src/.../nvml-wrapper-0.12.1`): `LIB_PATH = "libnvidia-ml.so.1"` on Linux (lib.rs:162) — good, because this machine has **no** bare `libnvidia-ml.so` (dlopen of it fails; `.so.1` works). Loading is via `libloading`, symbols resolve lazily (`nvml_sym` → `NvmlError::FailedToLoadSymbol` only when a missing function is used). `Nvml: Send + Sync` (static assert), `unsafe impl Send/Sync for Device<'_>`. `Nvml::init()` also reads the driver version to pick `FieldIdScheme::{V12, V13Update1}` (driver ≥ 580.82 renumbered field IDs 251–273; our 610 driver is V13Update1 and the crate translates transparently). Errors: `NvmlError::{NotSupported, NoPermission, NotFound, InsufficientSize(Option<usize>), GpuLost, LibRmVersionMismatch, LibloadingError, FailedToLoadSymbol, UnexpectedVariant(u32), ...}`; return codes 25/26/27 (ARGUMENT_VERSION_MISMATCH/DEPRECATED/NOT_READY) map to `UnexpectedVariant(n)`.

| nvtop/GPU-Z field | nvml-wrapper 0.12.1 call → C symbol | Result on this 5090 (driver 610) | cost |
|---|---|---|---|
| name | `Device::name()` → nvmlDeviceGetName | "NVIDIA GeForce RTX 5090" | static |
| driver | `Nvml::sys_driver_version()`; `sys_cuda_driver_version()` (v2) | 610.57.04; 13030 (CUDA 13.3) — nvidia-smi warns the old "Driver Version" is deprecated for CUDA 14 | static |
| GPU util / memctl util | `utilization_rates() -> Utilization{gpu,memory}` | 19 % / 5 % | 0.8 µs |
| VRAM | `memory_info() -> MemoryInfo{total,free,used,reserved,version}` (**v2**) | used 13856 MiB, reserved 460, total 32607 (v1 `used` would include reserved: 14316) | 3 µs |
| temp | `temperature(TemperatureSensor::Gpu)` | 45 °C | 0.4 µs |
| temp thresholds | `temperature_threshold(TemperatureThreshold::{Shutdown,Slowdown,GpuMax})` | 96 / 93 / 90 °C work; `MemoryMax`, `Acoustic*`, `GpsCurr` → `NotSupported`. Header says this API is "no longer preferred on Ada+"; the replacement `field_values_for(&[FieldId(NVML_FI_DEV_TEMPERATURE_SHUTDOWN_TLIMIT=193 / SLOWDOWN_TLIMIT=194 / GPU_MAX_TLIMIT=196)])` returns signed *margins* (-5, -2, 0) relative to T.Limit, and `nvmlDeviceGetMarginTemperature` (not wrapped) returns 45 | static |
| memory-junction temp | `field_values_for(&[FieldId(NVML_FI_DEV_MEMORY_TEMP=82)])` | inner `NotSupported` on GeForce (nvidia-smi `temperature.memory` = N/A) | – |
| fans | `num_fans()`=3; `fan_speed(i)` (FanSpeed_v2) = 30/30/31 %; `fan_speed_rpm(i)` (nvmlDeviceGetFanSpeedRPM, v1 struct) = 514/519 rpm | works; fan speed is the *intended* setpoint per header | 0.4 ms **per call** |
| power | `power_usage()` (1-s average on Ampere+ per header) = 108 W; `field_values_for(&[FieldId(NVML_FI_DEV_POWER_INSTANT=186)])` = 107.8 W (3 µs); `NVML_FI_DEV_POWER_AVERAGE=185` (3.3 ms) | all work | 0.4 µs / 3 µs |
| power limit | `enforced_power_limit()` 600 W (0.5 µs); `power_management_limit()` (122 µs); `power_management_limit_default()`; `power_management_limit_constraints() -> PowerManagementConstraints{min_limit,max_limit}` = 400–600 W | works | static-ish |
| energy | `total_energy_consumption()` mJ | works but **2.6 ms** | 1 s |
| clocks | `clock_info(Clock::{Graphics,SM,Memory,Video})` 2220/2220/7001/1807 MHz; `max_clock_info(..)` 3135/3135/14001/3090 | `applications_clock()`, `clock(_, ClockId::TargetAppClock)`, `max_customer_boost_clock()` → `NotSupported` (nvidia-smi: "deprecated") | 0.5 µs |
| PCIe link | `current_pcie_link_gen()`=5, `current_pcie_link_width()`=16, `max_pcie_link_gen()`=5, `max_pcie_link_width()`=16 | works; matches sysfs 32 GT/s x16 | 0.3–3 µs |
| PCIe RX/TX | `pcie_throughput(PcieUtilCounter::{Receive,Send})` KB/s | **blocks ~21 ms per call** (header: 20 ms byte-counter window). Alternative: `field_values_for(&[FieldId(NVML_FI_DEV_PCIE_COUNT_TX_BYTES=197), FieldId(198)])` returns monotonic byte counters in 0.2–0.4 ms — diff them yourself | 21 ms ×2 |
| enc/dec | `encoder_utilization()`/`decoder_utilization() -> UtilizationInfo{utilization,sampling_period}` | 0 % / 100 000 µs | 0.4 µs |
| pstate | `performance_state() -> PerformanceState` | P3 (enum has Zero..Fifteen, Unknown) | 0.3 µs |
| throttle | `current_throttle_reasons() -> ThrottleReasons` (bitflags; calls legacy `nvmlDeviceGetCurrentClocksThrottleReasons`, still exported by 610); `supported_throttle_reasons()` = 0x1ff | 0x0 now; nvidia-smi counters show 219 ms SW power capping | 0.4 µs |
| processes | `running_graphics_processes()` (**_v3**) → 7 procs; `running_compute_processes()` (_v3) → 2; `mps_running_compute_processes()`; `ProcessInfo{pid, used_gpu_memory: UsedGpuMemory::{Used(u64),Unavailable}, gpu_instance_id, compute_instance_id}` | per-process VRAM **is** populated on Linux/GeForce with 610 (gnome-shell 464 MiB, Cyberpunk 12579 MiB); Cyberpunk and ptyxis appear in both lists | 0.2 ms + 0.13 ms |
| per-process GPU% | `process_utilization_stats(last_seen_ts: impl Into<Option<u64>>) -> Vec<ProcessUtilizationSample{pid,timestamp,sm_util,mem_util,enc_util,dec_util}>` | **works on GeForce without root** (Cyberpunk sm 16–19 %, gnome-shell 2–4 %). Count query reports buffer capacity (72), real result 1–2 entries. Pass `last_seen = now_us - interval` to get only fresh samples; with 0 you get each process's last sample even if seconds old. Returns `Err(NotFound)` when nothing new → treat as empty | 1.7 ms |
| high-rate history | `samples(Sampling::Power, last_ts) -> Vec<Sample{timestamp, value: SampleValue::U32}>` | Power: 119 samples, **20 ms spacing, 2.36 s buffer**; GpuUtilization: 71 samples, 200 ms, 14 s; `ProcessorClock`/`MemoryClock`: count query succeeds but the fetch returns `NotSupported` | 1.4–1.6 ms |
| static IDs | `uuid()`, `pci_info() -> PciInfo{bus_id:"00000000:01:00.0", pci_device_id:0x2B8510DE, pci_sub_system_id, ..}` (v3), `vbios_version()` 98.02.2E.80.05, `brand()`=GeForce, `architecture()`=`DeviceArchitecture::Blackwell` (variant present, value 10), `cuda_compute_capability()`={12,0}, `num_cores()`=21760 (180 µs), `memory_bus_width()`=512 (190 µs), `bar1_memory_info()` 732/32768 MiB, `is_accounting_enabled()`=false | all work | static |
| process name | `Nvml::sys_process_name(pid, 64)` or `/proc/<pid>/cmdline` | | |

Not wrapped in 0.12.1 (but declared in nvml-wrapper-sys 0.9.1 bindings, callable through `device.nvml().lib().nvmlDeviceGetMarginTemperature` + `unsafe { device.handle() }`): `nvmlDeviceGetMarginTemperature`, `nvmlDeviceGetTemperatureV`, `nvmlDeviceGetCurrentClocksEventReasons`, `nvmlDeviceGetJpgUtilization`/`OfaUtilization`, `nvmlDeviceGetCoolerInfo`, `nvmlDeviceGetProcessesUtilizationInfo`. None are needed for nvtop parity.

## 3. Polling costs and threading

Measured on this machine via ctypes (same libnvidia-ml.so.1 the crate dlopens): `nvmlInit_v2` 4 ms, `GetHandleByIndex` 6 ms (once). Sub-microsecond: utilization, temperature, power usage, enforced limit, clock_info ×4, pcie gen/width, enc/dec, pstate, throttle reasons, POWER_INSTANT field. Tens–hundreds of µs: memory_info (3 µs), running processes (0.2 + 0.13 ms), fan speed/RPM (0.4 ms each ×3 fans ≈ 1.2 ms), power_management_limit (0.12 ms), BAR1 (84 µs), PCIe byte-counter fields (0.2–0.4 ms). Milliseconds: process_utilization_stats 1.7 ms, samples() 1.4–1.6 ms, total_energy 2.6 ms, POWER_AVERAGE field 3.3 ms, MEMORY_TEMP field 1.2 ms (even though unsupported — don't poll it), and `pcie_throughput` **21 ms blocking per direction**. Batched `field_values_for` costs the sum of the slowest members (5 IDs incl. 83+185 → 3.3 ms), so batch only cheap IDs together.

Recommended tiers:
- **Fast tick 250 ms (or 100 ms for a header gauge):** util, temp, power (usage or POWER_INSTANT), graphics/mem clocks, pstate, throttle bits — total well under 20 µs.
- **1 s tick:** memory_info, enc/dec, fans (%, RPM), processes + process_utilization_stats, PCIe link gen/width, PCIe byte counters (diff → B/s), energy, samples(Power) for a 50 Hz power trace.
- **2 s or on-demand only:** `pcie_throughput` if you want NVML's own 20 ms-window number; otherwise skip it in favour of the counters.
- **Once at start / on hotplug:** name, uuid, pci_info, arch, CC, cores, bus width, vbios, max clocks, max PCIe, limits/constraints, thresholds, num_fans.

All NVML calls must run on a dedicated worker (std thread is fine, matching astral-watch; tokio `spawn_blocking` also fine): a 21 ms PCIe call or a 3 ms field call would otherwise stall a 60 fps render loop. Shape:

```rust
struct GpuSnapshot { ts: Instant, util: u32, memctl: u32, temp_c: u32, power_mw: u32, limit_mw: u32,
    gclk: u32, mclk: u32, pstate: PerformanceState, throttle: ThrottleReasons, vram_used: u64, vram_total: u64,
    enc: u32, dec: u32, fans: Vec<(u32, u32)>, pcie: PcieState, procs: Vec<GpuProc>, power_trace: Vec<(u64,u32)>, /*..*/ }

fn spawn_gpu_worker(tx: watch::Sender<Arc<GpuSnapshot>>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new().name("gpu-nvml".into()).spawn(move || {
        let nvml = match Nvml::init() { Ok(n) => n, Err(e) => { tx.send_replace(Arc::new(GpuSnapshot::unavailable(e))); return; } };
        let dev = nvml.device_by_index(0).unwrap();          // Device<'_> borrows nvml; both live on this thread
        let static_info = read_static(&dev);                  // name, cores, bus width, max clocks, thresholds...
        let mut last_util_ts = 0u64; let mut prev_pcie = None;
        loop { /* fast fields every 250 ms; slow fields every 4th tick; publish Arc<GpuSnapshot> */ }
    }).unwrap()
}
```
Treat `Err(NotSupported)` per field as `Option::None` and stop polling that field; treat `Err(NotFound)` from `process_utilization_stats` as empty; on `GpuLost`/`LibRmVersionMismatch` mark the component degraded and retry init with backoff.

## 4. gpuwatch reference (third-party C++/FTXUI, NVML + SQLite)

`src/nvml_monitor.cpp` is a plain 1 s background thread calling exactly the nvtop set plus `nvmlDeviceGetTemperatureThreshold(SLOWDOWN)`, `nvmlDeviceGetFieldValues(NVML_FI_DEV_MEMORY_TEMP)`, `nvmlDeviceGetPowerManagementLimit`, `nvmlDeviceGetMaxClockInfo`, and PCIe gen/width/throughput. It links libnvidia-ml at build time (needs nvml.h from CUDA) — nvml-wrapper's dlopen is strictly better for opsTui. `gpu_database.cpp` looks up `data/gpu_specs.db` (44 rows, table `gpu_specs(name, pci_device_id, architecture, process, memory_type, compute_capability, cuda_cores, tmus, rops, sms, base_clock_mhz, boost_clock_mhz, memory_size_mb, memory_bus_width, tdp_watts, memory_bandwidth_gbs, l2_cache_mb, transistors_billion, die_size_mm2, rt_cores, tensor_cores)`) by PCI device id, falling back to a name LIKE match. The TUI shows a left "static" panel (arch/process/die/transistors, cores/SM/TMU/ROP/RT/tensor, base/boost, VRAM type/bus/bandwidth/L2, CC, TDP, PCI id) and a right live panel with an "OC delta" (max clock − spec boost) and live bandwidth from the current memory clock.

**Its RTX 5090 row is wrong in ways that matter:** pci_device_id is `2B06`, but the real 5090 is `10DE:2B85` (verified from `/sys/bus/pci/devices/0000:01:00.0/device`, `nvidia-smi pci.device_id 0x2B8510DE`, and `/usr/share/misc/pci.ids`: `2b85 GB202 [GeForce RTX 5090]`, `2b87 5090 D`, `2c02 5080`, `2c05 5070 Ti`, `2f04 5070`, `2d04 5060 Ti`, `2d05 5060`). Worse, the DB assigns `2B85` to "GeForce RTX 5070", so a PCI-id lookup would label this card a 5070; only the name fallback rescues it. Its 5090 L2 is 128 MB (that is the full GB202; the shipping 5090 has 96 MB) and die 744 mm² (NVIDIA/Wikipedia: 750 mm²). The other 5090 numbers match public specs and NVML: 21760 CUDA cores (NVML `num_cores` = 21760), 170 SM, 680 TMU/tensor, 176 ROP, 170 RT, 2017/2407 MHz, 32 GB GDDR7 512-bit (NVML bus width 512), 28 Gbps → 1792 GB/s, 575 W (this ASUS Astral has a 600 W default limit), 92.2 B transistors.

**Verdict:** worth embedding, but as a hand-verified `const` Rust table (~10 rows: 5090/5090D/5080/5070Ti/5070/5060Ti/5060, 4090/4080S/4080/4070TiS) keyed by PCI device id, not the SQLite file. Everything that NVML can supply live (name, cores, bus width, arch, CC, VRAM, max clocks, limits) should come from NVML and be cross-checked against the row (mismatch → show NVML value, flag the row). The table only adds what NVML cannot: SM/TMU/ROP/RT/tensor counts, L2, die, process, transistors, spec base/boost, GDDR data rate → theoretical bandwidth, TDP, launch/MSRP.

## 5. Size-class plan for the GPU component

- **1x1:** big-number `GPU 19%` with a 1-cell temp badge `45°`; colour by throttle bits/temperature; optional 1-row sparkline of util if height ≥ 3. Data: util, temp (fast tick).
- **2x1:** two gauges side by side: `GPU 19% | VRAM 42% (13.9/32.6G)`, second row `2220 MHz  108/600 W  45°C  30%` — the nvtop line-2 equivalent compressed; P-state and a `PWRCAP/THERM` chip when throttle bits are set.
- **4x2:** nvtop header parity: name/driver line, PCIe `GEN5@x16  RX/TX`, four bar gauges (GPU, MEMCTL, VRAM, ENC/DEC auto-hidden), clocks/temps/fans (3 fans %, RPM), power vs limit with a mini 20 ms-resolution power sparkline from `samples(Power)`.
- **6x3:** the above plus rolling charts (util, VRAM %, temp, power %, clock %, effective-load) with nvtop's 10-minute ring buffer, selectable series, and a right-hand static "GPU-Z" spec column from §4.
- **full:** charts + the process table (PID USER TYPE GPU% ENC DEC GPU-MEM CPU% HOST-MEM Command; sort, kill via `nix::sys::signal::kill`), sharing one `/proc` scan service with the htop component so CPU%/RSS are computed once.
- **astral-watch data:** merge as a **"Power" sub-panel** inside 4x2/6x3/full (board power from NVML on top, six per-pin amps bars + balance ratio + total W from `astral_watch::i2c::read_reading` beneath; the two numbers cross-check each other), **and** keep a standalone `Pin Power` component (1x1 total amps + balance colour, 2x1 six pin bars) for users who want it elsewhere in the grid. The i2c read runs on its own thread at ≥ 100 ms (astral-watch's own MIN_INTERVAL), never on the NVML worker, so a slow SMBus transaction cannot stall GPU sampling.

## 6. nvidia-smi subprocess fallback — when it is needed

Measured: `nvidia-smi --query-gpu=... --format=csv,noheader,nounits` ≈ 10 ms wall, 27 MB RSS; it is itself an NVML client, so it fails in exactly the situations NVML fails (`LibRmVersionMismatch` after a driver upgrade without reboot, no driver loaded). It therefore only helps when (a) opsTui is built without the `nvml` feature, (b) `Nvml::init()` returns `LibloadingError` because `libnvidia-ml.so.1` is not on the loader path while `nvidia-smi` is (containers, odd packaging — not this machine: `libnvidia-compute` provides `.so.1`), or (c) a field is missing from the wrapper and you would rather parse than call raw FFI. Keep astral-watch's `parse_gpu_csv` approach as the last tier at a 1–2 s cadence only; show an explicit "driver/library mismatch — reboot" state for `LibRmVersionMismatch` instead of silently falling back.

## Recommendations

- **Use nvml-wrapper 0.12.1 as the primary GPU backend, initialised once on a dedicated worker thread that owns `Nvml` and the `Device<'_>` handles and publishes immutable `Arc<GpuSnapshot>` values (tokio `watch` or a `Mutex<Arc<_>>`) to the render loop.** — Verified: default LIB_PATH is libnvidia-ml.so.1 (the only name present on this machine), Nvml is Send+Sync, Device is Send+Sync, init costs ~10 ms, and several calls block for 1–21 ms (pcie_throughput 21 ms per direction) which must never run on the render thread.
  - alternatives: Static linking against libnvidia-ml (needs nvml.h/CUDA at build time, breaks builds on non-NVIDIA CI); nvidia-smi parsing (10 ms fork per sample, same failure modes as NVML).
- **Split polling into three tiers: fast (250 ms, or 100 ms for header gauges) for util/temp/power/clocks/pstate/throttle; 1 s for memory_info, enc/dec, fans, processes + process_utilization_stats, PCIe byte counters, samples(Power); static once for name/cores/bus width/arch/CC/max clocks/limits/thresholds.** — Measured costs: fast-tier calls are 0.3–3 µs each; fan and process calls are 0.1–1.7 ms; energy/POWER_AVERAGE fields 2.6–3.3 ms; nothing in the fast tier exceeds 20 µs total.
  - alternatives: Single 1 s tick like nvtop/gpuwatch (simpler, but loses the 100 ms header responsiveness the user asked about).
- **Derive PCIe RX/TX from `field_values_for(&[FieldId(NVML_FI_DEV_PCIE_COUNT_TX_BYTES=197), FieldId(NVML_FI_DEV_PCIE_COUNT_RX_BYTES=198)])` byte counters diffed over the tick, and only call `pcie_throughput()` optionally at ≥2 s.** — The counters return in 0.2–0.4 ms; `nvmlDeviceGetPcieThroughput` blocks for a 20 ms sampling window per direction (header comment + measured 20.7/21.5 ms).
  - alternatives: Call pcie_throughput on its own thread; accept 42 ms/tick of blocked worker time.
- **Show both VRAM occupancy (used/total from memory_info v2, excluding `reserved`) and memory-controller utilisation (utilization_rates().memory), labelled distinctly (VRAM vs MEMCTL).** — nvtop's MEM bar is used/total (43 % observed) whereas NVML memory utilisation was 5 % at the same instant; conflating them is a common confusion.
  - alternatives: nvtop parity only (VRAM %).
- **Build the process table from `running_graphics_processes()` + `running_compute_processes()` (both _v3), tag TYPE by which list(s) a PID appears in (add a real 'G+C' type), overlay `process_utilization_stats(now_us - tick_us)` for GPU%/ENC/DEC, and get CPU%/RSS/user/cmdline from a shared /proc sampler (procfs 0.18 + uzers) reused by the htop component.** — Verified per-process VRAM and per-process SM% both work on this GeForce card with driver 610 without root; passing a recent lastSeen timestamp filters stale samples (with 0 you receive samples seconds old).
  - alternatives: Accounting APIs (`accounting_stats_for`) — accounting mode is disabled and needs root; nvidia-smi --query-compute-apps (compute-only, no graphics processes).
- **Embed a small hand-verified `const` spec table keyed by PCI device id (2B85 5090, 2B87 5090 D, 2C02 5080, 2C05 5070 Ti, 2F04 5070, 2D04 5060 Ti, 2D05 5060, 2684 4090, 2702 4080 SUPER, 2704 4080, 2705 4070 Ti SUPER) for SM/TMU/ROP/RT/tensor/L2/die/process/transistors/spec clocks/GDDR rate/TDP, and take everything else live from NVML, cross-checking `num_cores()` and `memory_bus_width()` against the row.** — gpuwatch's SQLite DB has the 5090 under the wrong id (2B06) and maps the real id 2B85 to 'RTX 5070', plus L2 128 MB and die 744 mm² errors; a 10-row Rust table with unit tests is smaller, auditable, and needs no sqlite dependency.
  - alternatives: Ship gpuwatch's gpu_specs.db via rusqlite (adds a C dependency and inherits its errors); no spec table at all (NVML alone gives name, cores, bus width, arch, CC, VRAM).
- **Use `temperature_threshold(Shutdown/Slowdown/GpuMax)` for the colour bands now (96/93/90 °C verified) but wrap it so the T.Limit field IDs (193/194/196 → signed margins) and `nvmlDeviceGetMarginTemperature` can replace it when NVIDIA removes the old API on Ada+.** — The nvml.h comment says the threshold API is no longer preferred on Ada and later and will be removed; the field-value replacement already works on driver 610.
  - alternatives: Hard-code 90/93 °C bands.
- **Treat nvidia-smi as a last-tier fallback used only when `Nvml::init()` fails with `LibloadingError` (or the nvml feature is off), at a 1–2 s cadence; on `LibRmVersionMismatch` show a 'driver/library mismatch, reboot' state instead.** — nvidia-smi is an NVML client with the same failure modes and costs ~10 ms + fork per sample; on this machine libnvidia-ml.so.1 is installed by the same driver packages as nvidia-smi.
  - alternatives: No fallback (simplest); always use nvidia-smi (astral-watch's current approach, fine for that tool but too slow for 250 ms ticks).
- **Add a 'Power' sub-panel to the GPU component that stacks NVML board power (usage + 20 ms `samples(Sampling::Power)` trace) above astral-watch's six per-pin amps/volts, balance ratio and alerts, and also expose the pin monitor as a standalone small component; run the i2c reader on its own thread at ≥100 ms.** — The two sources cross-validate (board W vs sum of pin W), the 50 Hz NVML power buffer (119 samples / 2.36 s verified) makes a visually rich trace, and astral-watch's library API (read_reading, Reading::total_watts/balance, alert::evaluate) is already published and MIT.
  - alternatives: Separate components only (loses the cross-check); merge everything into one panel (too dense for 2x1/4x2).

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `nvml-wrapper` | 0.12.1 | Safe NVML bindings; dlopens libnvidia-ml.so.1 at runtime (no headers, no link-time dependency); provides every call needed for nvtop parity plus fan RPM, samples(), field_values_for() with automatic CUDA-13U1 field-id remapping | none at build time; at runtime the NVIDIA driver's libnvidia-ml.so.1 (package libnvidia-compute-* on Ubuntu, present here). No root needed for any query used. | verified |
| `nvml-wrapper-sys` | 0.9.1 | Raw bindgen bindings pulled in by nvml-wrapper; exposes unwrapped symbols (nvmlDeviceGetMarginTemperature, nvmlDeviceGetCurrentClocksEventReasons, JPG/OFA utilisation) via `nvml.lib()` + `device.handle()` if ever needed | none | verified |
| `procfs` | 0.18.0 | /proc/<pid>/stat, statm, cmdline, status for the process table's CPU%, RSS, uid and command (shared with the htop component) | none | verified |
| `uzers` | 0.12.2 | uid → username for the USER column (getpwuid equivalent) | none | verified |
| `nix` | 0.31.3 | `nix::sys::signal::kill` for nvtop's F9 send-signal menu (or libc 0.2 `kill`) | none | verified |
| `sysinfo` | 0.39.6 | Alternative to procfs for per-process CPU/memory if the htop component already uses it; avoid double-scanning /proc | none | likely |
| `libc` | 0.2.x (cargo search shows a 1.0.0-alpha.4 prerelease; stay on the 0.2 line) | kill(2), sysconf(_SC_CLK_TCK/_SC_PAGESIZE) if not using nix/procfs | none | likely |

## Risks

- **Blocking NVML calls on the render thread: pcie_throughput blocks ~21 ms per direction, total_energy 2.6 ms, POWER_AVERAGE field 3.3 ms, process_utilization_stats 1.7 ms, fan queries 0.4 ms each.** → All NVML on a worker thread; use PCIe byte-counter fields instead of pcie_throughput; never poll unsupported fields (MEMORY_TEMP still costs 1.2 ms to return NotSupported).
- **NVIDIA is deprecating APIs used for parity: temperature thresholds via nvmlDeviceGetTemperatureThreshold on Ada+, applications clocks (already NotSupported on this card), and the old driver-version string (CUDA 14 will drop it).** → Wrap each behind a probe-once-and-cache strategy; prefer field IDs 193/194/196 and margin temperature; treat NotSupported/UnexpectedVariant(26 = DEPRECATED) as 'field absent', not as an error.
- **NvmlError::UnexpectedVariant for newer return codes (25 ARGUMENT_VERSION_MISMATCH, 26 DEPRECATED, 27 NOT_READY) and for enum values the wrapper does not know (future architectures beyond Blackwell).** → Match UnexpectedVariant explicitly; keep architecture display as a string from name()/pci id when architecture() fails.
- **process_utilization_stats returns Err(NotFound) when no process had non-zero utilisation since lastSeen, and with lastSeen=0 returns stale samples seconds old.** → Map NotFound → empty; pass now_us − tick_us; zero any process with no fresh sample (nvtop does the same).
- **gpuwatch's spec DB contains wrong PCI ids (5090 listed as 2B06; 2B85 mapped to 5070) and inflated L2/die numbers; copying it would mislabel the user's own card.** → Hand-curate a tiny table from pci.ids + Wikipedia/NVIDIA specs with unit tests; cross-check cores/bus width against NVML at runtime.
- **Driver upgrade without reboot → Nvml::init() fails with LibRmVersionMismatch (nvidia-smi fails identically, so a subprocess fallback does not help).** → Show an explicit degraded state with the reason and retry init with backoff; do not fall back to nvidia-smi for this case.
- **Memory-junction temperature and hotspot are simply unavailable on GeForce via NVML (field 82 → NotSupported); users coming from HWiNFO/GPU-Z on Windows may expect them.** → Render 'n/a' with a tooltip/help line rather than 0; the 12V-2x6 pin panel from astral-watch is the differentiator instead.
- **Per-process VRAM/utilisation visibility could regress on future drivers or in containers/namespaces (NVML reports host PIDs).** → Handle UsedGpuMemory::Unavailable and missing /proc entries gracefully; map PIDs via /proc only when present.
- **Multi-GPU layouts and MIG: utilisation/process calls are unsupported on MIG-enabled devices.** → Check mig_mode() once; skip those calls when MIG is active; render one header block per device like nvtop.

## Verified facts

- nvml-wrapper 0.12.1 Linux LIB_PATH is "libnvidia-ml.so.1" (local source ~/.cargo/registry/src/index.crates.io-*/nvml-wrapper-0.12.1/src/lib.rs line 162); this machine has libnvidia-ml.so.1 but no bare libnvidia-ml.so (ls + python ctypes dlopen test: .so fails, .so.1 succeeds).
- nvml-wrapper 0.12.1: Nvml is Send+Sync (assert_impl_all in lib.rs), Device<'_> has unsafe impl Send/Sync (device.rs:79-80); memory_info() calls nvmlDeviceGetMemoryInfo_v2 and returns MemoryInfo{free,reserved,total,used,version}; running_graphics/compute_processes() call the _v3 symbols; fan_speed(idx) uses FanSpeed_v2; fan_speed_rpm(idx) exists; samples(Sampling, Into<Option<u64>>) and field_values_for(&[FieldId]) -> Vec<Result<FieldValueSample>> exist; process_utilization_stats does the count query itself (grep of device.rs).
- nvml-wrapper 0.12.1 has FieldIdScheme {V12, V13Update1} chosen from the driver version (>= 580.82 → V13Update1) and translates field IDs 251-273; DeviceArchitecture::Blackwell exists (enums/device.rs:229, value NVML_DEVICE_ARCH_BLACKWELL=10).
- nvml-wrapper 0.12.1 NvmlError maps NVML codes up to VGPU_ECC_NOT_SUPPORTED; codes 25/26/27 (ARGUMENT_VERSION_MISMATCH/DEPRECATED/NOT_READY, defined in nvml-wrapper-sys 0.9.1 bindings) become UnexpectedVariant(n) (error.rs From<nvmlReturn_t>).
- Live on RTX 5090 / driver 610.57.04 via ctypes against libnvidia-ml.so.1: nvmlInit_v2 4.1 ms; GetHandleByIndex 6.3 ms; UtilizationRates 0.8 µs (gpu 19, mem 5); MemoryInfo_v2 3 µs (used 13856, reserved 460, total 32607 MiB; v1 used = 14316); Temperature 0.4 µs (45 C); PowerUsage 0.4 µs (108 W); EnforcedPowerLimit 0.5 µs (600 W); PowerManagementLimit 122 µs; constraints 400-600 W; ClockInfo 0.4-0.6 µs (2220/2220/7001/1807 MHz); MaxClockInfo 3135/3135/14001/3090; PcieThroughput 20.7 ms TX / 21.5 ms RX; PCIe gen 5 x16 current and max; Encoder/Decoder 0.4 µs (sampling period 100000 µs); PerformanceState P3; ClocksEventReasons 0x0, supported 0x1ff; NumGpuCores 21760 (182 µs); MemoryBusWidth 512 (190 µs); Architecture 10 (Blackwell); CC 12.0; Brand 5 (GeForce); VBIOS 98.02.2E.80.05; BAR1 732/32768 MiB (84 µs); TotalEnergyConsumption 2.6 ms.
- Temperature thresholds on this card: SHUTDOWN 96, SLOWDOWN 93, GPU_MAX 90 succeed; MEM_MAX, ACOUSTIC_MIN/CURR/MAX, GPS_CURR return NVML_ERROR_NOT_SUPPORTED (rc 3). Field IDs 193/194/196 (TEMPERATURE_SHUTDOWN/SLOWDOWN/GPU_MAX_TLIMIT) return signed -5/-2/0; 195 (MEM_MAX_TLIMIT) NotSupported; nvmlDeviceGetMarginTemperature returns 45 (0.56 ms); nvmlDeviceGetTemperatureV works (45).
- ApplicationsClock and MaxCustomerBoostClock return NOT_SUPPORTED for all clock types on this card; nvidia-smi reports 'Requested functionality has been deprecated' for applications clocks.
- NVML_FI_DEV_MEMORY_TEMP (82) returns inner NOT_SUPPORTED (call still costs ~1.2 ms); nvidia-smi temperature.memory = N/A. NVML_FI_DEV_POWER_INSTANT (186) works in 3 µs; POWER_AVERAGE (185) 3.3 ms; TOTAL_ENERGY (83) 2.3 ms; PCIE_COUNT_TX/RX_BYTES (197/198) monotonic counters in 0.2-0.4 ms; field 190 (POWER_CURRENT_LIMIT) = 600000 mW.
- Fans: NumFans = 3; FanSpeed_v2 per fan 30/30/31 % (~0.4-0.5 ms per call); FanSpeedRPM 514/519 rpm (0.37 ms); header comment says reported speed is the intended setpoint.
- Process lists on this card: GraphicsRunningProcesses_v3 → 7 processes with populated usedGpuMemory (gnome-shell 464 MiB, Xwayland 12, ..., Cyberpunk2077.exe 12579 MiB) in 196 µs; ComputeRunningProcesses_v3 → 2 (ptyxis 44 MiB, Cyberpunk 12579 MiB) in 129 µs; MPS → 0. nvidia-smi --query-compute-apps lists the same two.
- nvmlDeviceGetProcessUtilization works unprivileged on this GeForce: count query returns capacity 72 (rc INSUFFICIENT_SIZE), full call returns 2 samples with lastSeen=0 (gnome-shell sm 2-4 %, 3.9 s old; Cyberpunk sm 16-19 %, 80 ms old) in ~1.65 ms; with lastSeen = now-200 ms/1 s/2 s only the fresh Cyberpunk sample is returned.
- nvmlDeviceGetSamples: TOTAL_POWER 119 samples spanning 2.36 s (20.0 ms spacing, 99.5-120.6 W) fetched in 1.4 ms; GPU_UTILIZATION 71 samples over 14.0 s (200 ms spacing) in 1.6 ms; PROCESSOR_CLK count query succeeds (100) but the fetch returns NOT_SUPPORTED. nvidia-smi -q shows the same 119-sample/2.36 s power buffer.
- Driver: /proc/driver/nvidia/version = NVIDIA UNIX Open Kernel Module 610.57.04; nvmlSystemGetDriverVersion '610.57.04'; nvmlSystemGetCudaDriverVersion_v2 13030; nvidia-smi warns the 'Driver Version'/'CUDA Version' fields are deprecated for CUDA 14. Accounting mode disabled. No hwmon under /sys/bus/pci/devices/0000:01:00.0; hwmon list is nvme x3, mt7925, k10temp, r8169 x2, asus, spd5118 x2.
- GPU PCI identity: vendor 0x10de device 0x2b85 subsystem 1043:8a2e (sysfs); nvidia-smi pci.device_id 0x2B8510DE; /usr/share/misc/pci.ids: 2b85 GB202 [GeForce RTX 5090], 2b87 [RTX 5090 D], 2c02 GB203 [RTX 5080], 2c05 [RTX 5070 Ti], 2f04 GB205 [RTX 5070], 2d04/2d05 GB206 [5060 Ti/5060]. sysfs link: 32.0 GT/s x16 current and max.
- gpuwatch data/gpu_specs.db (44 rows, read-only sqlite via python) lists 'GeForce RTX 5090' with pci_device_id '2B06' (cores 21760, SMs 170, base 2017, boost 2407, 32768 MB, 512-bit, 575 W, 1792 GB/s, L2 128 MB, 92.2 B transistors, die 744 mm², 170 RT, 680 tensor, 680 TMU, 176 ROP) and maps '2B85' to 'GeForce RTX 5070'; gpu_database.cpp looks up by PCI id first, then LIKE on the name from 'RTX' onward.
- gpuwatch nvml_monitor.cpp polls once per second on a std::thread using GetUtilizationRates, GetMemoryInfo, GetTemperature, GetTemperatureThreshold(SLOWDOWN), GetFieldValues(NVML_FI_DEV_MEMORY_TEMP), GetFanSpeed, GetPowerUsage, GetPowerManagementLimit, GetClockInfo/GetMaxClockInfo (graphics, mem), GetPcieThroughput TX/RX, GetCurrPcieLinkGeneration/Width, GetEncoder/DecoderUtilization; CMake requires nvml.h from the CUDA toolkit and links libnvidia-ml.
- Wikipedia GeForce RTX 50 series table: RTX 5090 = GB202-300, 92.2 B transistors, 750 mm², 170 SM, 21,760 CUDA, 680 TMU, 176 ROP, 680 tensor, 170 RT, L2 96 MB, 2.01/2.41 GHz, 32 GB GDDR7 512-bit 28 Gb/s, 1792 GB/s, 575 W, $1,999, 30 Jan 2025 (WebFetch).
- nvtop 3.2.0 installed; --help lists -d delay (tenths of s), -c config, -p no-plot, -P no-processes, -r reverse, -C no-color, -f fahrenheit, -i gpu-info bar, -E encode-hide seconds (default 30), -s snapshot; `nvtop --snapshot` took 0.06 s and printed gpu_util 19 % / mem_util 43 % (VRAM occupancy) while NVML memory utilisation was 5 %.
- nvtop sources: plot ring buffer `interface_alloc_ring_buffer(devices_count, 4, 10 * 60 * 1000, ...)` (10 minutes); effective_load = gpu_util_rate * power_draw / power_draw_max; process CPU% = 100*(Δuser+Δkernel)/Δwall from /proc/<pid>/stat; GPU MEM % = used/total; NVIDIA backend uses ProcessUtilization with lastSeenTimestamp and counts NULL-buffer INSUFFICIENT_SIZE; TYPE never set to 'both' by the NVIDIA backend; gpu-info bar fields not populated for NVIDIA; keybindings F2/F6/F9/F12/F10/q/Esc/+/-/arrows (README/manpage/interface.c/interface_setup_win.c/extract_gpuinfo*.c via WebFetch).
- nvml.h (libnvidia-ml-dev, /usr/include/nvml.h) comments: PcieThroughput 'querying a byte counter over a 20ms interval'; PowerUsage 'On Ampere (except GA100) or newer ... averaged over 1 sec'; ProcessUtilization returns NVML_ERROR_NOT_FOUND when no samples since lastSeenTimeStamp; GetSamples lastSeenTimeStamp semantics; TemperatureThreshold 'no longer the preferred interface ... on Ada and later ... use nvmlDeviceGetFieldValues with NVML_FI_DEV_TEMPERATURE_* fields'; FanSpeed is the intended speed.
- nvidia-smi --query-gpu CSV subprocess costs ~10 ms wall / 27 MB RSS here (/usr/bin/time x3); astral-watch's tui.rs polls it every ~1.5 s with query pci.bus_id,utilization.gpu,power.draw,power.limit,temperature.gpu,fan.speed and parses '[N/A]' as None; astral-watch's optional `safety` feature already depends on nvml-wrapper 0.12.1 via Nvml::init() + device_count/device_by_index (Cargo.toml, src/safety.rs).
- cargo search (today): nvml-wrapper 0.12.1 (rust-version 1.60, features legacy-functions, serde), nvml-wrapper-sys 0.9.1, procfs 0.18.0, uzers 0.12.2, nix 0.31.3, sysinfo 0.39.6, libc shows 1.0.0-alpha.4 as the latest published (prerelease).

## Open questions

- Grid unit size: the size classes above assume roughly 1 unit ≈ 20 columns × 6 rows; confirm the real cell geometry before fixing what fits in 1x1/2x1.
- Whether the per-process CPU%/RSS sampler should live in a shared 'process service' crate used by both the htop and GPU components (recommended) — depends on how the htop component is designed.
- Whether to expose NVML's 20 ms power samples as a first-class 'power trace' widget (needs a 1 s poll of samples(Power) with the last timestamp carried forward) or only as a sparkline in the Power sub-panel.
- Is fan RPM worth the 3 × 0.4 ms per second (fan_speed + fan_speed_rpm for three fans ≈ 2.4 ms/s of worker time)? Cheap enough, but confirm the user wants RPM next to %.
- Process kill UX: nvtop offers signals 1–31; opsTui may want only TERM/KILL/INT plus confirmation — product decision.
- How far to go on the static spec table (Blackwell + Ada only, or also Ampere) and whether to include board-specific data (ASUS Astral 600 W limit, 12V-2x6) keyed by subsystem id 1043:8A2E.
- Behaviour when the astral-watch i2c bus is unreadable (user not in i2c group, module unloaded) inside the merged Power sub-panel — degrade the panel to NVML-only or hide the pin rows?
- The T.Limit field IDs 193/194/196 return margins (-5/-2/0) rather than absolute temperatures; confirm the intended display (e.g. 'slowdown at +2 °C above T.Limit') if you migrate off temperature_threshold().

## Sources

- Local: /home/mattbeam/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nvml-wrapper-0.12.1/src/{lib.rs,device.rs,error.rs,enum_wrappers/device.rs,enums/device.rs,struct_wrappers/device.rs,structs/device.rs,bitmasks/device.rs}
- Local: /home/mattbeam/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nvml-wrapper-sys-0.9.1/src/bindings.rs
- Local: /usr/include/nvml.h (libnvidia-ml-dev), /usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1 (nm -D), /proc/driver/nvidia/version, /proc/driver/nvidia/gpus/0000:01:00.0/information, /sys/bus/pci/devices/0000:01:00.0/*, /usr/share/misc/pci.ids
- Local: ctypes timing/probe runs against libnvidia-ml.so.1 (this session), nvidia-smi --query-gpu/--query-compute-apps/-q outputs, nvtop --help / --snapshot
- Local: /home/mattbeam/workspace/gpuwatch/{README.md,CMakeLists.txt,src/nvml_monitor.cpp,src/nvml_monitor.h,src/gpu_specs.h,src/gpu_database.cpp,src/gpu_database.h,data/gpu_specs.db}
- Local: /home/mattbeam/workspace/astral-watch/{Cargo.toml,src/tui.rs,src/safety.rs,src/i2c.rs}
- https://docs.rs/nvml-wrapper/0.12.1/nvml_wrapper/struct.Nvml.html
- https://docs.rs/nvml-wrapper/0.12.1/nvml_wrapper/device/struct.Device.html
- https://docs.rs/nvml-wrapper/0.12.1/nvml_wrapper/error/enum.NvmlError.html
- https://docs.rs/crate/nvml-wrapper/0.12.1/source/src/lib.rs
- https://github.com/Cldfire/nvml-wrapper/blob/main/CHANGELOG.md
- https://raw.githubusercontent.com/Syllo/nvtop/master/src/extract_gpuinfo_nvidia.c
- https://raw.githubusercontent.com/Syllo/nvtop/master/src/extract_gpuinfo.c
- https://raw.githubusercontent.com/Syllo/nvtop/master/src/interface.c
- https://raw.githubusercontent.com/Syllo/nvtop/master/src/interface_setup_win.c
- https://raw.githubusercontent.com/Syllo/nvtop/master/src/get_process_info_linux.c
- https://raw.githubusercontent.com/Syllo/nvtop/master/manpage/nvtop.in
- https://raw.githubusercontent.com/Syllo/nvtop/master/README.markdown
- https://github.com/Syllo/nvtop/releases/tag/3.2.0
- https://docs.nvidia.com/deploy/nvml-api/group__nvmlDeviceQueries.html
- https://docs.nvidia.com/deploy/nvml-api/known-issues.html
- https://en.wikipedia.org/wiki/GeForce_RTX_50_series
- cargo info nvml-wrapper / nvml-wrapper-sys; cargo search uzers, nix, procfs, libc, sysinfo (crates.io, 2026-08-30)
