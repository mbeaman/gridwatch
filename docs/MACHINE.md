# torch — the machine gridwatch is built against

Verified 2026-08-30 with read-only commands. Re-verify anything marked *(assumption)* before relying on it.

## Hardware and OS

- **CPU:** AMD Ryzen 9 9950X3D2 — 16 cores / 32 threads, two CCDs (`k10temp` exposes `Tctl`, `Tccd1`, `Tccd2`). RAPL package power at `/sys/class/powercap/intel-rapl:0` (`package-0`); `energy_uj` is **root-only (0400)** — a udev rule is the documented fix.
- **Memory:** 91 GiB. DIMM temperatures via `spd5118` hwmon ×2.
- **GPU:** ASUS ROG Astral GeForce RTX 5090 — PCI `0000:01:00.0`, device `10de:2b85`, subsystem `1043:8a2e`, driver **610.57.04**, 32 607 MiB, power limit 600 W (range 400–600 W). `libnvidia-ml.so.1` at `/usr/lib/x86_64-linux-gnu/` (nvml-wrapper dlopens it). `nvidia-smi` present.
- **Storage:** 3× NVMe (hwmon `nvme` ×3: Composite / Sensor 1 / Sensor 2).
- **Other hwmon:** `mt7925_phy0` (wifi), `r8169` ×2 (NICs), `asus` (no readings). **No fan/pump sensors** (no `nct67xx` driver loaded). `lm-sensors` not installed.
- **OS:** Ubuntu 26.04.1 LTS, kernel 7.0, GNOME. Docker and libvirt installed (bridges present).

## Network

- `eno1` — r8169 2.5GbE, **UP**, default route; `eno2` down; `wlp7s0` (mt7925) down; `virbr0`, `docker0`, `br-*`, `veth*` bridges/pairs (filtered by default in the net component).
- `net.ipv4.ping_group_range` covers the user → unprivileged ICMP DGRAM sockets work (no CAP_NET_RAW needed).

## Audio and media

- **PipeWire 1.6.2** with WirePlumber, `pipewire-pulse` (native socket at `/run/user/1000/pulse`) and `pipewire-alsa`. Graph quantum default 1024 (a running game pins it lower).
- Sinks: USB Audio DAC — Speakers (60), **Front Headphones (61, default)**, S/PDIF (59); NVIDIA HDMI (75). Sources: USB Line In (62), Microphone (63).
- CLI available: `pw-record`, `pw-cat`, `pw-mon`, `pw-top`, `pw-dump`, `wpctl`, `arecord`, `ffmpeg`. **Not** available: `pactl`/`parec`, `cava`, `playerctl`.
- Verified capture line: `pw-record --format f32 --rate 48000 --channels 2 --raw --latency 1024 --target auto -P '{ stream.capture.sink = true, node.passive = true, node.name = "gridwatch audio" }' -` (2400 frames → 19 200 bytes).
- **MPRIS** on the session bus: `org.mpris.MediaPlayer2.firefox.instance_*`; `mpris-proxy` (Bluetooth AVRCP) running.

## Terminal

- **Ptyxis 50.1** on **VTE 0.84.0** (`VTE_VERSION=8400`), `TERM=xterm-256color`, `COLORTERM=truecolor`. VTE draws box drawing (U+2500–259F), sextants and **octants (U+1CD00–1CDE5) natively**; braille comes from font fallback (DejaVu Sans / Noto Symbols2).
- Fonts installed: DejaVu Sans Mono, Noto Sans Mono, Liberation Mono, Nimbus Mono. **No Nerd Fonts** → `nerd` glyph tier stays off.
- Sixel: `libvte` contains Sixel symbols; whether Ptyxis enables it: see the note at the bottom.
- Kitty keyboard protocol: not supported by VTE — legacy key encoding only.
- **Ptyxis is a GPU client:** `nvidia-smi` lists `/usr/bin/ptyxis` (PID 11805) as `C+G` with 44 MiB — terminal output is composited on the GPU, so redraw volume is GPU load by proxy (see `PERFORMANCE.md` P9/P10).
- **Window size: unknown** *(assumption: 250×70 used throughout the plan)* — measure with the arc-1 `F12` HUD.

## Toolchain

- **Rust 1.95.0 stable** (cargo, clippy, rustfmt, miri, rust-analyzer). MSRV target for the project: **1.88** (ratatui 0.30.2 floor). Cargo-installed: `amdgpu_top`, `astral-watch`, `wasm-pack`. Not installed: cargo-nextest, cargo-deny, cargo-audit, cargo-insta, just.
- Python 3.14.4, Node 22.22. No Go, Bun or Zig.
- `build-essential`, `pkg-config` present. **Missing dev headers:** `libasound2-dev`, `libpipewire-0.3-dev`, `libpulse-dev`, `libdbus-1-dev` — the design avoids every crate that needs them.
- `gh` 2.86 logged in as **mbeaman**. crates.io reachable.
- Reference tools: `htop` 3.4.1, `nvtop` 3.2.0.

## Permissions

- User `mattbeam` groups include **`i2c`** (so `/dev/i2c-*`, `root:i2c 0660`, is readable without sudo), `video`, `render`, `docker`, `libvirt`, `kvm`, `sudo`.
- RAPL `energy_uj`: root-only. `/proc/<pid>/io`, `exe`, `fd` of other users' processes: unreadable (rendered `N/A`, as htop does).

## astral-watch on this machine

- Checkout at `../astral-watch`, HEAD `dce7eee77676268c66b3624c7a2870ed9d84eb9c` (v0.7.0 + two unreleased commits: advisory imbalance split, i2c block read). Not published on crates.io.
- Service **not installed** here (`astral-watch.service` not found), no `/etc/astral-watch.toml`, no user config, exporter not listening → the pins source will take the **direct i2c** path unless you run the exporter.
- The chip only answers with plausible telemetry under GPU load; at idle expect `NoTelemetry` / `TelemetryLost`.

## Sibling reference

- `../gpuwatch` — third-party C++/NVML GPU-Z-style tool (MIT), cloned as a reference for the GPU spec panel. Its bundled DB mislabels the 5090; the plan uses a small hand-verified `const SPECS` table instead.

## Sixel note

`libvte-2.91-gtk4.so.0` exports `vte_terminal_set_enable_sixel` (Sixel is compiled in), but `/usr/bin/ptyxis` contains no reference to that API and Ptyxis has no `sixel` setting, and VTE's `enable-sixel` property defaults to **off**. Net: **no Sixel in Ptyxis today**; the half-block album-art painter is the right default, and Sixel remains a possible later opt-in only if Ptyxis (or another terminal) enables it.
