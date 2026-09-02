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
- **exec-plugin security posture** — arc 8 must spec input limits (max line length, schema-validated only, reject unknown kinds), resource caps (kill on runaway CPU/RSS), and no shell interpretation of plugin commands.
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
- **Per-source/per-component Cargo features**: `WORKSPACE.md` plans one feature per source and component, but `procfs` is an unconditional dependency and both registries register unconditionally, so the CI feature matrix has nothing to select. Do it when the second source lands (arc 2) or amend `WORKSPACE.md` §15 to say features arrive then.
- **Frame counting is coupled to changed-cell accounting**: `--stats-log` and the F12 HUD both clone the frame buffer and diff 17 500 cells per frame, so P-rows measure the product plus its instrument (`docs/PERFORMANCE.md`, arc-1b notes). Split the frame counter from the cell diff so P4's 0.3 % row can be taken with frame counts.
- **Resize has no automated test** (raised by Matt, 2026-09-01): the renderer *is* dynamic — a live pty resized six times mid-run (60×20 → 200×45 → 40×8 → 250×50 → 158×1 → 131×38) re-derived the mode, re-solved the grid and re-picked every tier correctly, and the too-small notice fired at one row — but nothing in CI would catch that breaking. Drive one `Shell` through a resize sequence in `crates/app/tests/shell.rs` and assert the mode, the tier and the notice at each size. Offered at the end of the arc-1b session and not yet written.
- **`/proc/stat` CPU lines are positional**: entry *i* is assumed to be CPU *i*, which an offline CPU would break while `cpu.topology` still indexed by CPU id. Torch never offlines a CPU; revisit with the hotplug path in arc 2.
