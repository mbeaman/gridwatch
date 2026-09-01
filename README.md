# gridwatch

A modular, themeable ops dashboard for the terminal — a grid of components that each render from a 1x1 tile to full screen, in themes like `retrowave` and `modern`, reproducing the behaviour of htop, nvtop and [astral-watch](https://github.com/mbeaman/astral-watch) alongside network, audio-visualizer and Winamp-style now-playing tiles. Rust + ratatui 0.30, built for a single Linux workstation first.

**Status:** arc 1 (`v0.1.0`) — the grid lights up. The core seam, the 12×6 layout engine, three themes, the cpu source and the htop tile are in; the GPU, pins, network, audio and Winamp tiles arrive in later arcs (see [`docs/ROADMAP.md`](docs/ROADMAP.md)).

```
 gridwatch  1 Overview  2 Audio   retrowave · configured
┏ CPU ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓ ╔ GPU ══════════════════════════════════════════════════════════╗
┃CPU [||||||||||||||||||||||                             ] 43.9%┃ ║                                                               ║
┃MEM [|||||||||||||||||||                          ] 16.7G/91.0G┃ ║                                                               ║
┃SWP [                                             ] 0.00K/16.0G┃ ║                                                               ║
┃641 pids, 2433 tasks; 3 running · load 4.51 4.20 3.52          ┃ ║                                                               ║
┃CCD0  4.9 GHz  61.8 °C         CCD1  5.4 GHz  53.6 °C          ┃ ║                                                               ║
┃                                                               ┃ ║                                                               ║
┃   ▁      ▄ ▆▃       ▆                                         ┃ ║                                                               ║
┃▃▇ █  ▅▃  █ ██ ▅  ▇▂ █▂                                        ┃ ║ ▪ gpu                                                         ║
┃██ █▆ ██ ▇█ ██ ██ ██ ██                                        ┃ ║ arrives in a later arc                                        ║
┃██ ██ ██ ██ ██ ██ ██ ██                                        ┃ ║                                                               ║
┃██ ██ ██ ██ ██ ██ ██ ██                       ▄▂               ┃ ║                                                               ║
┃██ ██ ██ ██ ██ ██ ██ ██        ▄▅ ▄▅    ▇▆    ██     ▃         ┃ ║                                                               ║
┃██ ██ ██ ██ ██ ██ ██ ██        ██ ██ ▆█ ██ ██ ██ ▄▄ ▁█         ┃ ║                                                               ║
┃0  1  2  3  4  5  6  7         8  9  10 11 12 13 14 15         ┃ ║                                                               ║
┃PSI cpu 1.37 · mem 0.19 · io 1.09                              ┃ ║                                                               ║
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛ ╚═══════════════════════════════════════════════════════════════╝

╔ PINS ═══════════════════════════════════╗ ╔ NET ════════════════════════════════════╗ ╔ AUDIO ══════════════════════════════════╗
║                                         ║ ║                                         ║ ║                                         ║
║                                         ║ ║                                         ║ ║                                         ║
║                                         ║ ║                                         ║ ║                                         ║
║                                         ║ ║                                         ║ ║                                         ║
║ ▪ pins                                  ║ ║ ▪ net                                   ║ ║ ▪ audio                                 ║
║ arrives in a later arc                  ║ ║ arrives in a later arc                  ║ ║ arrives in a later arc                  ║
║                                         ║ ║                                         ║ ║                                         ║
║                                         ║ ║                                         ║ ║                                         ║
║                                         ║ ║                                         ║ ║                                         ║
╚═════════════════════════════════════════╝ ╚═════════════════════════════════════════╝ ╚═════════════════════════════════════════╝

╔ SOURCES ════════════════════════════════╗ ╔ SENSORS ══════════════════════════════════════════════════════╗
║cpu ok                                   ║ ║                                                               ║  00:00
║                                         ║ ║ ▪ sensors                                                     ║
║                                         ║ ║ arrives in a later arc                                        ║
╚═════════════════════════════════════════╝ ╚═══════════════════════════════════════════════════════════════╝
 q quit · ? help · [ ] pages · hjkl focus · Enter capture · z zoom · d dense · t theme · space pause · S shot · F12 hud
```

*`gridwatch shot --format ansi --size 131x37 --theme retrowave`, colour stripped for this page — the real frame is a truecolor synthwave palette with gradient-coloured core bars. A captured screenshot lives in `docs/img/` once a human has taken one.*

## Run it

```sh
cargo run --release                 # live: the Overview against this machine
cargo run --release -- run --demo   # synthetic data, no hardware needed
cargo run --release -- shot --format ansi --size 250x70 --theme retrowave
cargo run --release -- config default > ~/.config/gridwatch/config.toml
cargo run --release -- doctor       # what this machine can feed
```

Keys: `1`–`9` pages, `Tab`/`hjkl` focus, `Enter` capture, `z` zoom, `d` dense,
`t` theme, `space` pause, `F12` stats HUD, `?` help, `q` (or `Ctrl-q`) quit.

## What is here today

- **`htop` tile** — htop 3.4.1's meters with its formulas verbatim (guest
  subtracted, `cached = Cached + SReclaimable − Shmem`, iowait counted as idle),
  32 gradient core bars grouped into real CCD blocks from sysfs `die_id`, per-CCD
  frequency and `Tccd` temperature, load, tasks and PSI — from an 8×3 chip to a
  full screen. Parity is tracked row by row in [`docs/PARITY.md`](docs/PARITY.md).
- **`clock` and `sources` tiles**, the second of which shows every source's
  state, generation, age, drops and restarts.
- **Themes** `retrowave`, `modern`, `mono` — components never name a colour.

## Where to read next

- [`docs/PLAN.md`](docs/PLAN.md) — the call, decisions, arc table
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — the design and its contracts
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — arcs, deliverables, acceptance
- [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) — the ceilings and what was measured
- [`docs/PARITY.md`](docs/PARITY.md) · [`docs/THEMES.md`](docs/THEMES.md) — tool parity and glyph support
- [`docs/WORKSPACE.md`](docs/WORKSPACE.md) — crate layout and dependencies
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — decision log
- [`docs/MACHINE.md`](docs/MACHINE.md) — the machine it is built against

MIT licensed.
