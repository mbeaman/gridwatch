<!-- The notes attached to a draft release. A person edits this before
     publishing; what is here is the part that is true of every build. -->

## Installing

Download the tarball for your libc, or the `.deb` / `.rpm`:

```sh
tar xzf gridwatch-*-x86_64-unknown-linux-gnu.tar.gz
sudo install -m755 gridwatch-*/gridwatch /usr/local/bin/
gridwatch doctor          # what this machine can do, and the fix for what it cannot
gridwatch run --demo      # the whole dashboard on synthetic data
```

Checksums are beside each archive (`sha256sum -c`).

## Which build

- **gnu** is the one you want. It `dlopen`s NVML when it is there, so the GPU
  tile works against your installed driver.
- **musl** is static and will run anywhere, and **cannot load NVML** — a static
  binary has no dynamic loader. The GPU tile falls back to `nvidia-smi`, and
  `gridwatch doctor` reports `✗ Nvml` rather than pretending. If you have an
  NVIDIA card, take the gnu build.

## What it needs

Nothing beyond libc. Every source is optional and degrades with a reason on
screen: no PipeWire means no visualizer, no session bus means no now-playing
tile, no NVIDIA card means no GPU tile. `gridwatch doctor` lists each with the
command that fixes it.

The package **ships** a udev rule for RAPL package power at
`/usr/share/gridwatch/udev/` and does not install it: reading the CPU's energy
counter is a permission decision that belongs to you, not to an installer. The
file explains itself.

## Configuration

```sh
mkdir -p ~/.config/gridwatch
gridwatch config default > ~/.config/gridwatch/config.toml
```

Both files hot-reload while it runs. `gridwatch config check` validates them.
