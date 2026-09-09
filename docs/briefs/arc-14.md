# Arc 14 brief — "what the drives are doing" (D64)

> Written 2026-09-09 with D64. **Fable was rate-limited, so an Opus session wrote the design**; read D64 first — it decides everything below and this brief only says where each piece goes. **Arc 14 changes no seam.** No new `Unit`, `Capability`, `View` variant, `Control`, `Manifest` field or `Detail` use. If anything here seems to need one, **stop and escalate** — do not design one. Build **14a, then 14b**, sequentially.

## What already exists (do not rebuild)

| thing | where | state |
|---|---|---|
| the typed option `Reader` and `SourceDef.check` | `sources/src/options.rs`, `store/src/source.rs` | landed (D63) — the disk source uses it like the other seven |
| `LabelSet::Dynamic` + per-label eviction | `store/src/key.rs`, `Store::sweep` | landed (D61) — every labelled `disk.*` key is `Dynamic` |
| the glob rule | `store::rules::glob` | landed (D57) — `extra`, `devices` and `hide` all use it |
| `ChipInfo.device` (`nvme0`, `0:0:0:0`) | `store/src/keys/sensors.rs`, computed by `sources/src/sensors/hwmon.rs` | landed — **this is the join key; add nothing to the sensors side** |
| the demo chip inventory with `nvme0/1/2` | `store/src/demo/sensors.rs` | landed — the join is testable under `--demo` with no hardware |
| the `Roots { proc, sys }` pattern | `sources/src/net/mod.rs` | copy it |
| the sparkline that holds, `assert_grows_with_area` | `ui::renderer`, testkit | landed (D62) — the chart window is the run's age capped at the span |
| the pty sandbox with `XDG_CONFIG_HOME` | `cli/tests/pty.rs` | landed — acceptance places `disk` here, **not** in the shipped `layout.toml` |

## 14a — the data

**Seam 1 — keys (`store/src/keys/disk.rs`, `SOURCE = SourceId("disk")`).** The eleven keys of §8 verbatim, with the `Unit` and `LabelSet` columns exactly as listed (`disk.queue` is **`Count`**, not `Ratio`). `DiskInfo` gets `Serialize + Deserialize`, a `decode` entry in `METAS` and a journal exemplar; the round-trip test over every Record `KeyMeta` picks it up automatically. **No aggregate keys** (D64 §2).

**Seam 2 — `demo::DiskSynth` (`store/src/demo/disk.rs`), and it carries the join.** Three drives whose `disk.info{…}.device` is `nvme0` / `nvme1` / `nvme2`, matching `demo::sensors_info()` exactly, so the cross-source temperature join is exercised by `--demo`, every snapshot and CI. Byte-deterministic per `(seed, Ts)` like the other six. **One drive must be busy** (a plausible read/write mix with a queue that rises) and **one must be idle with no completions**, so the `—` await path has data. `disk_info() -> SourceInfo` beside `net_info()`, `disk_demo` for `SourceDef.demo`.

**Seam 3 — the source (`sources/src/disk/{mod,stat,sysfs}.rs`, feature `disk`, no new dependency).** `Roots { proc, sys }`. `stat.rs` parses `/proc/diskstats` into `Counters` and `Counters::rates(prev, dt)` in the shape of `net/dev.rs`; `sysfs.rs` classifies one device name. Cadence `hidden max(refresh, 2 s) / visible refresh / focused max(refresh/2, 250 ms)`, `always_on: false`. `OPTION_NAMES = ["refresh_ms", "partitions", "extra"]`, read in that order via `Reader` (`int_ms("refresh_ms", 250..=10_000, "", 1000)`, `bool("partitions", false)`, `str_list("extra", true, &[])`), `check = Options::from_table(t).1`. Register on `SourceDef` in `sources/registry.rs`. `requires: &[]`, `produces: &["disk.*"]`.

**The per-key contract, once:**

| key | from | note |
|---|---|---|
| `read_bps` / `write_bps` / `discard_bps` | Δ index **5** / **9** / **16** × 512 / Δt | diskstats sectors are always 512 B whatever `logical_block_size` says |
| `reads_ps` / `writes_ps` | Δ index **3** / **7** / Δt | |
| `busy_pct` | 100 × Δ index **12** / Δwall_ms, clamped 0–100 | `io_ticks` |
| `queue` | Δ index **13** / Δwall_ms | `time_in_queue`; `iostat`'s `aqu-sz` |
| `read_await_ms` / `write_await_ms` | Δ index **6** / Δ index 3, Δ index **10** / Δ index 7 | **published only when the completion delta is > 0** |
| `info` | sysfs, once per name | published on **change** (net's `links()` diff) |
| `scan_ms` | the pass's wall ms | `Label::None`, `Static` |

> Indices are **0-based after splitting the line**. `/proc/diskstats` is documented 1-based *including* major, minor and name, so field 13 (`io_ticks`) is index 12 and field 14 is index 13. Get this wrong by three and every number is plausible and wrong.

**Seam 4 — the device rule (`sysfs.rs`), D64 §3 and §5 verbatim.** A drive = a `/sys/block` entry with a `device` link, `hidden == 0`, `size > 0`. A partition = a diskstats name that is not a `/sys/block` entry (published only under `partitions = true`). `extra` = globs. `MAX_DEVICES = 16`, filled drives → partitions → extras, refusals in `SourceStatus.reason`. **Classify lazily**: on first sight of a name, cached, dropped when the name leaves diskstats. No rewalk timer.

**Seam 5 — features.** `sources/Cargo.toml` `disk = []`; `components/Cargo.toml` `disk = []`; `app/Cargo.toml` `disk = ["gridwatch-sources/disk", "gridwatch-components/disk"]`; `cli/Cargo.toml` `disk = ["gridwatch-app/disk"]`; add to all four `default` lists **and to CI's feature matrix**. Regenerate `docs/KEYS.md`.

**Fixtures (`fixtures/diskstats/torch/`).** A recorded `proc/diskstats` (the real 64-line, 4 205-byte file) and a `sys/block` tree with `nvme0n1` (+ its two partitions and its `device` link, `hidden`, `size`, `queue/{rotational,scheduler,nr_requests}`, `device/model`), `nvme1n1`, `loop0`, **`loop2` (`size = 0`)**, and a hand-made `md0` (virtual, no `device` link — the `extra` case). Plus a 17-field pre-5.5 diskstats for the degrade.

**Tests for 14a.** In `stat.rs`: the 20-field parse; a 17-field line degrading to no `discard_bps` and never being skipped; a 24-field future line ignoring its tail; a counter reset yielding 0, not a negative or a huge number; `busy_pct` clamped; the `queue` formula against a hand-computed case; the await pair absent when the completion delta is 0. In `sysfs.rs`: **3 drives of 55 entries** from the fixture tree, `loop2` refused for `size = 0`, a `hidden = 1` device refused, a partition refused unless asked for, `extra = ["md*"]` admitting `md0`, and the cap dropping extras before partitions and partitions before drives. In `mod.rs`: a device that vanishes stops being remembered; the ask-set tripwire `Reader::asked() == OPTION_NAMES`; a rejected value naming the real default.

## 14b — the tile

**Seam 6 — the component (`components/src/disk/{mod,view}.rs`, feature `disk`).** The manifest, five tiers, options and column-drop order of §8, verbatim. `OPTION_NAMES = ["devices", "hide", "sort", "series"]`. **`demand(tier) = Detail::Meters` at every tier including `full`** — this is the arc's cheapest review check, so make it one obvious line. `tick` rebuilds the model when the source's last sample moves (net's pattern); `view` never sorts.

**Seam 7 — the temperature join.** `disk.info{d}.device` → the `sensor.info` chip with the same `device` → `sensor.temp_c{<chip>:Composite}` (falling back to the chip's first `temp` label), plus `sensor.crit_c` for the band. **Never by hwmon index or chip suffix** — on torch the two numberings disagree, and any agreement is a coincidence of devpath sorting. `optional_sources = [sensors]`. `—` plus a reason in `full` for each of: no sensors source, no chip with that `device`, a chip with no temperature.

**Seam 8 — the await staleness rule.** Draw the last await while it is within **3 × the source's cadence** (§11's existing constant), else `—`. Do **not** compare the await sample's `Ts` to the source's last sample — a drive at 1–2 IOPS would strobe every second. The label stays alive for D61's sweep because `read_bps` publishes `0.0` on every tick regardless; assert that in a store test.

**Tests for 14b.** `snapshot_matrix!` at the tiers; `assert_renders_everywhere` with a `signature(tier)` (D46 — "didn't panic" is never a passing assertion); `assert_min_tier_fits`; `assert_tiers_well_formed`; **`assert_grows_with_area`** (D62) over every listed tier, with any honest exclusion named and numbered in the test's comment as arc 12 did. A tier-per-rect test at 17×8 / 38×8 / 80×20 / 122×31 / 59×18 / 39×11 and zoomed 248×66. The join test from `demo::DiskSynth` + the sensors synth (a temperature per drive; `—` with the sensors series absent). A column-drop test at 36 / 41 / 48 / 55 / 61 / 80 wide. **The disjointness test gains `disk` to its table, prints `disk×disk` and `disk×sensors` in `checked`, and adds nothing to `KNOWN`** — a hard requirement of the arc.

Pty **C.36**: a sandbox `layout.toml` placing `disk`, `--demo` draws the three synthetic drives with rates and a temperature, `1`–`4` change the chart series, `q` exits 0. Pty **C.37**: `[sources.disk] refresh_ms = "1000"` fails `config check` with D63's sentence and toasts once at `run`.

**Docs for 14b.** The D64 ARCHITECTURE edits are already applied; 14b adds `PERFORMANCE.md` P23 and the re-taken rows, regenerates `COMPONENTS.md` and `KEYS.md` via `scripts/shots.sh`, adds a `disk` placement example to `docs/LAYOUT.md`, and writes `CHANGELOG.md`, `PLAN.md` and `BACKLOG.md` (the `P1` struck as pulled, plus the four new entries: filesystem capacity, SMART, htop's NetworkIO sum, the diskstats merge fields).

## Numbers

- **P23 (new).** The pass ≤ 1 ms wall, ≤ 0.2 % of one core at 1 s. An upper bound is already taken: `/proc/diskstats` is **4 205 bytes / 64 lines** on torch and costs **34.9 µs** to read and **69.5 µs** to read and parse all fields of all lines — in **Python**, over 5 000 iterations, so it is a ceiling and the Rust figure is this arc's to measure into `disk.scan_ms`. Comparables: the `/proc/*/fd` connection scan is 3.3 ms, the pid-level scan 5.4 ms.
- **P5 is the row that can break.** 33 wake-ups/s measured with every source live, ceiling 40. Disk adds 1/s visible (2/s focused), and 1 s aligns with `net` and `sensors` while 500 ms aligns with `gpu` and `pins`, so `next_deadline` may absorb most of it. **Measure it; do not assert it.** If it exceeds 40, the disk cadence is the knob.
- **P17.** 3 drives × 9 scalars × 38 400 B ≈ **1.0 MB**; `partitions = true` on torch (12 devices) ≈ **4.2 MB**; the 16-device cap ≈ **5.5 MB**; all 64 diskstats lines ≈ **22 MB** against P17's measured **40.3 MB** of a 60 MB budget. The last number is the justification for the whole device rule — put it in the CHANGELOG too.
- **P1, P6, P8, P19** re-taken with the disk tile visible. **P13 and P15 unchanged**, and asserted so: the disk source raises no `Detail`.
- **Cross-check.** Idle: `iostat -x 1 2` beside the tile. Loaded read side: one bounded `dd if=<file> of=/dev/null iflag=direct bs=1M count=512` (≈ 0.5 s, no CPU or GPU load — inside MACHINE.md). **The write-side cross-check is Matt's row** — an agent does not write half a gigabyte to his boot drive beside a game.

## Traps — do not rediscover

1. **52 loop devices, not eight** — and their *rates are zero*. The cost is series and rows, not accuracy.
2. **Field indices are off by three** if you read the docs as 0-based. `io_ticks` = index 12.
3. **Sectors are always 512 B** in diskstats, whatever `logical_block_size` says.
4. **`canonicalize` errors** on a virtual device's missing `device` link — that is the *signal*, not a failure to log.
5. **hwmon numbering is not drive numbering.** Join by `device`, never by index or suffix.
6. **Two namespaces share one controller temperature** — correct, and the pane says the reading is the controller's.
7. **`disk.info` on change only**, or a Record lands every tick and the journal grows for nothing.
8. **The awaits strobe** if you compare timestamps. Use §11's 3 × cadence rule.
9. **The chart window is the run's age capped at the span**, never a fixed span (D62 amendment 1).
10. **Never raise `Detail`.** Any tier returning more than `Meters` is a bug in this arc.
11. **§9 disjointness**: the source has no `devices` key and the tile has no `chips`, `rapl` or `refresh_ms`, on purpose. Adding one puts a fifth entry in `KNOWN` (D61) and fails the arc.
12. **Do not touch the shipped `layout.toml`.** Acceptance uses the pty sandbox.

## Escalate rather than improvise

A new `Unit`, `Capability`, `View` variant, `Control` or `Manifest` field. Any use of `Detail` above `Meters`. A `SetOption` control for `partitions`. Reading `/dev/nvme*`, `statvfs`, a mount table, or anything requiring a capability. A crate outside `WORKSPACE.md`'s table.

## Gates

`scripts/gate.sh` green (`scripts/shots.sh` regenerates `KEYS.md` **and `COMPONENTS.md`** — the drift step fails until both are regenerated); commit before review; the review workflow from `docs/REVIEW.md` with the read-only guard in every agent prompt and a lens that greps the component for `std::fs` (a component may not do I/O, §4.6) and for `Detail::`; scoped commits (`store:`, `sources:`, `components:`, `docs:`), 14a's before 14b's so each half reverts alone; push `main` and watch CI. `v0.14.0` and the `Cargo.toml` version are Matt's.

## Done when

**14a:** `gridwatch keys` lists eleven `disk.*` rows with `dynamic` labels; the source publishes three drives on torch and no loop devices; `extra = ["loop*"]` publishes 16 devices and says what it refused; `refresh_ms = "1000"` fails `config check`; P23 is recorded. **14b:** a sandbox layout's `disk` tile shows the three drives with plausible rates, a `BUSY` column, a `Q` column and a temperature joined from the sensors source; the same tile shows `—` for the temperature with `--no-default-features --features disk`; the growth sweep passes for every listed tier; the disjointness test names `disk×disk` and `disk×sensors` and adds nothing to `KNOWN`; P5 and P17 re-taken. **Matt's rows:** the write-side `iostat` cross-check under real load, the tag, and where `disk` lives in the shipped default layout.
