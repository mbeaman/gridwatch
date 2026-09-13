# Installing

## What you need

**Linux**, and a **Rust toolchain 1.88 or newer**. That is the whole list —
there are no `-dev` packages to install, no system libraries to link against.
Everything gridwatch reads, it reads from `/proc`, `/sys`, a D-Bus socket, or a
library it opens at runtime if it happens to be there.

If you do not have Rust, [rustup.rs](https://rustup.rs) installs it in a minute:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Install

```sh
cargo install --git https://github.com/mbeaman/gridwatch
```

`--git` is not a typo, and plain `cargo install gridwatch` will not work.
gridwatch depends on astral-watch by git revision and crates.io forbids git
dependencies, so `--git` is the supported path until astral-watch is published.

## The first two commands

```sh
gridwatch run --demo    # synthetic data — no hardware, nothing to configure
gridwatch               # the real thing, against this machine
```

Start with `--demo`. It is seeded and deterministic: every tile has plausible
data, alerts fire, the audio visualizer moves, and nothing on your machine is
touched. It is the fastest way to see what the thing is before deciding whether
you want it.

Press `?` for every key. `hjkl` moves between tiles, `z` zooms one to full
screen, `t` cycles themes, `e` edits the layout, and `q` quits — press `Esc`
first if you have handed the keys to a tile with `Enter`, because a tile that
holds the keys keeps `q` too.

## What your hardware decides

Most tiles need nothing but `/proc` and `/sys`. Four want something more, and
**a tile that cannot reach its hardware says so rather than drawing a zero**:

| Tile | Needs | Without it |
|---|---|---|
| **GPU** | NVIDIA's `libnvidia-ml.so.1` (the driver's compute library) | The tile names the missing library and the package that carries it. There is no AMD or Intel path — the tile is NVML-only. |
| **Audio** / **Now playing** | PipeWire's `pw-record` on `PATH`; a player on the session bus for MPRIS | The audio tile says `pw-record` is missing; the now-playing tile says there is no player. |
| **Pins** | An ASUS 12V-2x6 card with [astral-watch](https://github.com/mbeaman/astral-watch) watching its sensor, reached over its exporter or `/dev/i2c-*` | The tile says which of the three backends it tried. |
| **Sensors** (package power) | A udev rule opening `/sys/class/powercap/*/energy_uj`, which is root-only on most kernels | Temperatures still work; the RAPL package-power row is a dash. |

Two of these want you in a group or a rule in place rather than new hardware:

```sh
sudo modprobe i2c-dev              # then, for the pins tile:
sudo usermod -aG i2c $USER         # log out and back in
```

## When something is blank, run this first

```sh
gridwatch doctor
```

It prints every capability gridwatch knows how to use, whether it found it, why
not if it did not, and the exact command that would change the answer — the same
sentences a placeholder tile shows in miniature. `--offline` skips the probes
that touch hardware.

---

Next: **[The tiles](Tiles.md)**, or **[Configuring](Configuring.md)** if you
already want to rearrange things.
