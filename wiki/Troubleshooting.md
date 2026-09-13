# Troubleshooting

## Run this first

```sh
gridwatch doctor
```

Every capability gridwatch knows how to use, whether it found it, why not if it
did not, and the command that would change the answer. `--offline` skips the
probes that touch hardware.

A tile whose source cannot work shows the same two lines in miniature — the
reason and the fix — instead of a number. **That is the tile working correctly.**

## A tile is blank or shows a dash

A dash means *not available*, never zero. The common causes:

| Symptom | Cause | Fix |
|---|---|---|
| GPU tile says the library is missing | NVIDIA's `libnvidia-ml.so.1` is not installed | Install the driver's compute library — on Debian/Ubuntu, `libnvidia-compute-*` |
| Audio tile is empty | `pw-record` is not on `PATH` | Install PipeWire's CLI tools. gridwatch does not link any audio library, so this is the only dependency |
| Audio tile is present but still | There is no sound playing | Correct — it animates only while there is sound, and stops the capture child ten seconds after you look away |
| Pins tile cannot find the sensor | `i2c-dev` is not loaded, or you are not in the `i2c` group | `sudo modprobe i2c-dev` then `sudo usermod -aG i2c $USER`, and log out and back in |
| Sensors tile has temperatures but no package power | `energy_uj` is root-only on most kernels | A udev rule opening `/sys/class/powercap/*/energy_uj` to your user |
| A drive's temperature is a dash | The sensors source is not running, or it is a SATA drive with no `drivetemp` | Add a sensors tile, or `modprobe drivetemp` |
| Now-playing tile says there is no player | Nothing is on MPRIS | Firefox counts; so does most of KDE and GNOME's media stack |
| htop's `GPU%` column is a dash | It reads DRM fdinfo, which NVIDIA's proprietary driver does not expose | Use the [GPU tile](Tiles-GPU.md), which reads NVML |

## There is no disks tile

It ships, but nothing places it. See
[Configuring → adding the disks tile](Configuring.md#adding-the-disks-tile).

## A number stopped moving

Look at the [Sources](Tiles.md#sources) tile. It shows each source's state, how
old its newest sample is, what it has dropped and how many times it has
restarted — which distinguishes "the source died" from "the value genuinely is
not changing".

A reading that has gone stale is badged `STALE` rather than left as a flat line
that reads as steady.

## My config change did nothing

```sh
gridwatch config check
```

It validates both files — option **types** as well as names — and exits non-zero
naming the key, the bad value and the fix. The common one is quoting a number:
`refresh_ms = "1000"` is a string, and used to be silently ignored.

`[store] history` is the exception that needs a restart, and gridwatch says so
rather than pretending it took effect.

## Tiles look empty on a very wide terminal

They should not any more. If you find a tile that does not fill the room it has,
that is a bug worth reporting — two arcs of work went into exactly this, and the
current rule is that a drawing takes its size from the rect it is given while
text is sized by its content. Include your terminal dimensions; `gridwatch shot
--size WxH` reproduces any size headlessly.

---

*This page is a stub and will grow as real reports come in. If something here
did not help, the reason is worth adding.*
