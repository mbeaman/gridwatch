> **Status: created 2026-08-31 (D38) — the single home for everything known but not scheduled.** Buckets: *Pre-flight* (before arc 1a), *Scheduled* (lives in ROADMAP, listed here only as a pointer), *Backlog* (wanted, unscheduled — pull into an arc via a DECISIONS entry), *Won't do* (recorded so it isn't re-litigated). Migrate to GitHub issues once the repo exists, keeping this file as the index.

# Backlog

## Pre-flight (before arc 1a — minutes, not sessions)

- [x] **Create the repo** *(done 2026-08-31)*. `git init` in `~/workspace/opsTui`, first commit = the planning corpus (docs/, CLAUDE.md, README.md) with `.gitignore` (`/target`, `/.cargo/config.toml`, `*.log`); create `github.com/mbeaman/gridwatch` (default: public, like astral-watch — Matt's call at creation) and push. `LICENSE` = MIT (matches `[workspace.package]`).
- [x] **Directory-name decision** *(decided 2026-08-31: repo created in place; dir rename deferred, memory-migration note stands)*. The dir is `opsTui`, the repo is `gridwatch`. Default: create the repo in-place and defer any rename — renaming the directory re-keys Claude's per-project memory path (`-home-mattbeam-workspace-opsTui`); if renamed later, copy `~/.claude/projects/-home-mattbeam-workspace-opsTui/memory/` to the new key first.
- [x] **CHANGELOG.md scaffold** *(done 2026-08-31)* (`[Unreleased]` header) so arc commits have somewhere to write.

## Scheduled (pointers — the roadmap owns these)

Arc 2 journal + CI screenshots · arc 3 astral-watch v0.8.0 pre-req + capability probe/doctor · arc 4b matrix tuning (fade/trail/sweep/density vs usability — explicitly expected to iterate, D35) · arc 8 pins CSV/multi-card, exec plugin host, htop/nvtop full parity · arc 9 packaging, theme import, docs completion.

## Backlog — components and features

- **`disk` component** — per-NVMe throughput/IOPS/util (a new read — nothing in the plan touches `/proc/diskstats` today, and htop's DiskIO/NetworkIO header meters are marked OUT in PARITY until this lands) + the nvme hwmon temps; tiers rates → sparks → per-device table. The catalogue currently has no disk I/O view; htop parity implies one. *(candidate: arc 7 companion or its own mini-arc)*
- **`power` combined tile** — one tile stacking RAPL package W + GPU board W + pins total W (+ wall estimate); the cross-check of NVML vs pin sum is already in the gpu `full` sub-panel — this makes it a first-class 2x1/4x2. *(cheap once sensors + pins exist)*
- **Out-of-band Crit alerting (off by default)** — `notify-send` on a Crit raise while the terminal is unfocused, so a pins overload isn't only a banner nobody is watching during a game. The stance itself gets **recorded as a DECISIONS entry in arc 3** when the overlay ships (D39): the **astral-watch service remains the real alerter**; gridwatch is a viewer.
- **Journal time-travel** — interactive scrub over a recording (pause/seek/speed keys on `--replay`), building on the virtual clock.
- **State persistence** — remember last page/zoom/theme across runs (`~/.local/state/gridwatch/state.toml`).
- **Multi-GPU rendering** — keys are `{dev}`-labelled from day one (D24); the gpu component renders one device until a second card exists to test against; nvtop-style stacked device blocks then.
- **`gridwatch plugin verify <cmd>`** — a contract-test subcommand for exec-plugin authors, replaying schema fixtures against their process. *(arc 8+)*
- **Container/VM tile** — docker/libvirt awareness (running containers/VMs, their CPU/mem) — torch runs both; nothing in the catalogue touches them.
- **Config migration** — `gridwatch config migrate` when `schema` bumps past 1 (`config.toml` carries `schema = 1` from arc 1a; a migration writes `~/.config/gridwatch/*.bak` first).
- **Nerd-glyph tier docs** — the `nerd = true` glyph tier exists but no Nerd Font is installed; document the install path for users who want it.
- **Upstream `Segmented`/stacked-bar widget** to ratatui-widgets or tui-widgets once proven in-tree.
- **Machine-readable output** — `--once`/`--json` and/or a Prometheus exporter (the prior-art digest's pattern #10, dropped from the plan without a decision until now); the store makes it cheap; decide scope when someone actually wants it.
- **`gridwatch-netd` privileged helper** — per-process network bandwidth (pcap/eBPF, CAP_NET_RAW); the net tile shows a capability badge until this exists (closes ARCHITECTURE's dangling "helper arc" reference).
- **htop header-meter breadth** — DiskIO/NetworkIO meters, configurable meter sets, and the Bar/Text/Graph/LED meter modes (the LED mode is a retrowave gift); PARITY marks them OUT until pulled.
- **gridwatch on crates.io** — blocked twice over: crates.io forbids git dependencies, and astral-watch's own crates.io publish was deprioritized. Publishing gridwatch requires publishing astral-watch 0.8.0 first; until then `cargo install --git` is the documented path (§14).

## Backlog — hardening and environment

- **Focus events under tmux/ssh** — the P4/P21 unfocused throttle rides DECSET 1004; verify behaviour inside tmux and over ssh, and define the safe degrade (no focus events ⇒ assume focused; `space` remains the manual throttle). *(verify during arc 1a's P21 check)*
- ~~**exec-plugin security posture**~~ — **done, arc 8b**: no shell, `env_clear`, a 1 MiB line cap, schema validation before anything is read, unknown kinds refused, three strikes, `RLIMIT_AS`/`RLIMIT_CPU`, a 64-deep drop-oldest queue, a 500 messages/s read budget and a 50 %-of-a-core-for-10 s runaway check (P22, D58 amendments 28–30).
- **Wayland clipboard for `S` screenshot** (copy path or content) — nice-to-have.

## Won't do (recorded)

- htop delay-accounting columns (CPUD%/IOD%/SWPD%) — needs `task_delayacct=1` + taskstats netlink + CAP_NET_ADMIN; htop itself shows N/A on torch. Render N/A forever.
- cdylib/abi_stable in-process plugins — no stable Rust ABI (D32); the wire protocol is the extension path.
- Sixel/kitty album art on this terminal — Ptyxis doesn't enable Sixel (MACHINE.md); half-blocks stand. Revisit only if the terminal changes.
- A second rendering stack for WASM plugins pre-1.0 — post-1.0 host crate at most (D32).
- Winamp realism beyond MPRIS: no real EQ (would need a PipeWire filter-chain), no true bitrate/kHz metadata (derived from `audio.sink`), no MPRIS TrackList playlist (players don't implement it — the playlist is local metadata history). The EQ weights the visualizer bands and is otherwise decorative; recorded so arc 6 isn't reviewed against a bar the platform cannot meet.

- **Dense border merging** (arc 4): `Block::merge_borders(MergeStrategy::Exact)` for the one-cell-overlap shared borders (§6, deferred by D42).
- **`shot --config`**: screenshots of the user's own layout (shot is pinned to embedded defaults for §12.5 determinism since the arc-1a review).
- **Control redelivery across source restarts**: a `SetOption` sent in the instant between supervisor generations is dropped (D42); queue-and-replay if a real source ever cares.

Raised by the arc-1b review (2026-08-31), verified real, deliberately not fixed in 1b:

- **Segmented meters are unreadable in `mono`** (arc 4, theme/renderer): every segment draws the same `|` glyph, so with no colour the MEM bar's used/shared/buffers/cache boundaries vanish and it reads far fuller than it is. The fix belongs to the renderer (a per-segment glyph in the `mono` widget set), not to a component.
- **The key bar is clipped mid-word below ~118 columns** (arc-1a code): it is one fixed string. Build it from `(key, label)` pairs and drop whole entries from the right the way `htop::view::info_line` already does for its clauses.
- **`[sources.<id>]` options are not validated**: a mistyped `refres_ms` is silently ignored, where a mistyped *component* option is a build error (`deny_unknown_fields`). Needs a per-source options type or a key-set check at load; §9 only mandates the disjointness rule, which is tested.
- ~~**Per-source/per-component Cargo features**~~ — done in arc 2a (D47 seam 5): `cpu`/`htop` features, `gpu` declared for 2b, the workspace root forwards them, CI's matrix checks none / each / all.
- **Frame counting is coupled to changed-cell accounting**: `--stats-log` and the F12 HUD both clone the frame buffer and diff 17 500 cells per frame, so P-rows measure the product plus its instrument (`docs/PERFORMANCE.md`, arc-1b notes). Split the frame counter from the cell diff so P4's 0.3 % row can be taken with frame counts.
- **`USER` column: htop's own-uid shading** needs the cpu source to publish its uid (a component may not call `getuid`, §4.6); arc 2a mutes root-owned rows instead. Elevated-capability colouring needs `status` (arc 8).
- **Journal pause marker**: `r` pauses the tee but leaves no line in the file, so a `--speed 1` replay of a paused stretch sleeps through the gap as if the machine stalled; a `st` line for the journal source at pause/resume would let readers skip it.
- **Demand changes never wake a parked source** (§4.3 `sleep_until` re-enters its park on a non-Stop control): zooming into a table tier can show "waiting for the process scan" for up to a scan period. A wake control that shortens the park is a `sleep_until` contract change — escalate when it matters.
- **`mono`'s table is one flat colour**: the unit ladder (M/G/T), state colours and the muted `0.0` are all colour; the selected row is now reverse video, the rest wants the per-glyph treatment the segmented meters also need (above).
- **`torch-game.jsonl` fixture** (whenever Matt has the game up): `gridwatch run --record fixtures/journals/torch-game.jsonl` for 60 s at 250×70 with tables off; 2a and 2b could only record the idle box (no heavy jobs from an agent, MACHINE.md). With it: re-take P1/P6 beside the game, and P11's fast tier under load (D49 §6 — the idle card makes `utilization_rates` 0.65 ms).
- **nvtop chart series not offered** (arc 2b): encoder/decoder rate, fan speed and memory clock are plottable in nvtop; gridwatch offers util/vram/temp/power/clock/load. Cheap to add to `gpu::view::series_points` if wanted (PARITY marks them out).
- **PCIe link maximum from the gpu source** (`gpu.pcie_gen_max`/`width_max` via NVML, or a spec-row field — a seam addition): the pins header's `↓` means "below Gen5×16" until then (arc 3a review).
- **Per-bar role override for `View::Bars`** so the pin bars' fill could follow the amps band (red over 9.2 A) as tui.rs does; today the fill is the `Power` gradient by height and the bands colour the values row (arc 3a review; a `View` change).
- **pins CSV-tail backend** (arc 8): the digest's third mode — `stat`-poll a root logger's rotating CSV, local naive timestamps, `chrono` for freshness; deferred by D50 §5 because the i2c path works beside that logger.
- ~~**gpu `full` tier's nvtop keys**~~ — **done, arc 8a**: `+`/`−` sort direction, `F9` signal menu and `h`/`l` column scroll all ship; the Power sub-panel's pin bars landed with arc 3.
- **The nvidia-smi fallback tier is untested live** (arc 2b): torch loads `libnvidia-ml.so.1`, so `gpu::smi` only has its parser under test. Exercise it in a container or by hiding the library once; the same goes for `LibRmVersionMismatch` and `GpuLost`.
- **Table paging when zoomed**: `PgUp`/`PgDn` and selection-follow in both tables page by the grid's row budget even in the zoomed body. The stated blocker is gone — `InputCx` has carried `zoomed` and `tier` since arc 8a (D58 amendment 7) — so this is now a small change nobody has made rather than a seam addition. htop has the same limit.
- **`config check` does not build components**, so an option `run` rejects (`sort = "nonsense"`) passes the check — an arc-1 gap the 2b review surfaced.
- **`samples(Power)` runs for any visible gpu tile** (D49 §2), 0.65 ms/s; a `Demand` hint finer than `Detail` would let a badge-only layout skip it. Escalate if a layout ever needs the 0.65 ms.
- **`/proc/stat` CPU lines are positional**: entry *i* is assumed to be CPU *i*, which an offline CPU would break while `cpu.topology` still indexed by CPU id. Torch never offlines a CPU; revisit with the hotplug path in arc 2.
- **Rule state is never evicted** (arc 7b review): `Rules::states` holds one small entry per rule and label. That is bounded by the label set, which is static for every catalogued key — except `net.*{iface}` on a machine that creates and destroys interfaces (veth, tun, docker), where a `*`-labelled rule accrues one entry per interface ever seen. `Store::series` has the same shape and no eviction either, so the fix belongs with retention, not in the rules engine.
- **IPv4 addresses per interface** (arc 7a review): `Link.addrs` carries only the IPv6 addresses, from `/proc/net/if_inet6`. v4 addresses are not in procfs — `ip addr` reads them over netlink and `getifaddrs` needs `unsafe`, which every crate forbids. Needs a decision: a minimal netlink client in `sources`, or an `unsafe` exemption for one `getifaddrs` call.
- **Wi-Fi details have no live path** (arc 7a): `WifiInfo` is carried by the key and drawn by the tile, and nothing fills it. `wlp7s0` is down on torch, so an agent can neither write nor test an nl80211 path honestly. Matt's row.
- **Multi-card pins** (arc 8a, deferred with reason — D58 amendment 6): the tile should tab between cards, but every `pins.*` key is labelled by pin number alone, so a card dimension is a change to the whole key seam (`pins.amps{card:pin}`), the exporter and CSV parsers, the alert ids and the demo synth. Torch has one card, so none of it could be verified here. It needs its own brief, not a mid-arc improvisation.
- **htop's userland thread rows** (arc 8a): `H` raises the source's demand to `Detail::Columns` and the gated files are read, but the `task/` walk that would *list* each thread as a row is not implemented — the toggle currently changes what is asked for, not what is shown. The demand path and its cost are proven (P15's gated row); the walk itself is the remaining work.
