# gridwatch — the plan

> **Status: arc 1 is built, committed and pushed (2026-09-01) — `main` on `github.com/mbeaman/gridwatch`, CI green on all eight jobs. `v0.1.0` is deliberately NOT tagged yet: P4/P21/P9/P10 need a human in Ptyxis (see `PERFORMANCE.md`'s owed list), as do the glyph check and the README PNG. Arc 2 is next; its brief is written (`docs/briefs/arc-2.md`, D47) — start session 2a from it.** Repo: `github.com/mbeaman/gridwatch` (the on-disk directory remains `~/workspace/opsTui`; see `CLAUDE.md` for why). This file is the entry point — read it first in every session, then `ROADMAP.md` for the current arc and the tail of `DECISIONS.md`.

## What we are building

A personal ops dashboard for the terminal on **torch** (RTX 5090 ROG Astral, 9950X3D2, Ubuntu 26.04, PipeWire, Ptyxis): a **grid of components** you can add, remove and rearrange, where every component renders well from a **1x1 tile to full screen**, in **file-defined themes** (retrowave, modern, phosphor, …). Components reproduce the functional behaviour of **htop**, **nvtop** and **astral-watch** (per-pin 12V-2x6 amperage — your own crate), plus **network monitoring**, an **audio visualizer** and a **Winamp-style now-playing tile** driven by MPRIS. Built in Rust over multiple sessions, one arc ≈ one minor version, pushed to a new repo under `github.com/mbeaman`.

## The call

| Area | Decision | One-line why |
|---|---|---|
| Stack | **Rust 1.88+ (edition 2024), ratatui 0.30.2, crossterm 0.29** | You already write ratatui (astral-watch's TUI); it is the only compiled toolchain on the box; NVML, i2c, procfs, D-Bus and FFT are all pure-Rust or dlopen. |
| Shape | **Six-crate workspace**: `store ← ui ← components ← app ← bin`, `store ← sources ← app` | Enforced boundaries make adversarial review mechanical: crossterm only in `app`/`bin`; components never touch a device or a thread; the store builds in ~1 s and holds most testable logic. |
| Modules | **Plugins against two contracts** — `Source` (data in) and `Component` (view out) — with a `Manifest` and a `Registry`; static in-process now; a **component returns a semantic view tree** and the theme's renderer draws it (themes own form via `[widgets]`, not just paint); the `exec` JSON-lines protocol (arc 8) is the public plugin API, schema-validated, WASM later | Rust has no stable ABI, so in-process plugins are compile-time; a plugin that returns a view tree instead of cells cannot break the theme, the layout or the readability rules. |
| Data | **Single-writer typed `Store`** owned by the render thread; sources publish `Batch`es over a bounded data channel, alerts/status over an unbounded **control** channel drained first | No locks on the data path, alarms can never be dropped, recording is a channel tee, replay is deterministic. |
| Layout | **Fixed 12 × 6 unit grid** per page; footprints (`1x1 2x1 4x2 6x3`) are picker hints; each component declares **cumulative tiers with minimum inner sizes** and the *real* rect picks the tier; `dense` and `stack` modes derived from terminal size with hysteresis | Measured on a 250×70 terminal: 1x1 = 17×8 inner (square in pixels), 4x2 = 80×20, 6x3 = 122×31. Every tier-0 fits 8×3, so nothing is ever a useless chip above stack mode. |
| Themes | **TOML themes**: semantic roles, `$palette`, Oklab gradient LUTs, glyph tiers (`ascii`/`unicode`/`nerd`), border + title specs, flourishes, effect hooks (tachyonfx, arc 4), and a **class**: `quiet` or `showcase`; built-ins `modern` (Catppuccin Mocha), `retrowave`, `mono`, `terminal`, `phosphor-green/amber`, and **`matrix`** — where **only the rain draws**: the rendered frame is a mold the rain falls through — katakana trails in empty space (fast fade), the module's own characters where it crosses content (slow fade to black), chrome included — so a widget is the rain's memory of having fallen through its shape; a dense sweep re-prints the page every ~20 s, changed values re-light themselves, a governor keeps it inside its own CPU/terminal budget, and it freezes the moment you tab away; alerts and the focused tile are continuously printed | Components never name a colour or glyph, so one render path yields every theme; `unicode` tier is safe with your DejaVu-class fonts (no Nerd Fonts installed; half-width katakana fall back to Noto Sans Mono CJK JP, verified). |
| Components | `htop`, `gpu`, `pins`, `net`, `audio`, `winamp`, `sensors` + free `clock`, `sources`, `alerts` | Each has 4–6 tiers from badge to tool-parity; `docs/PARITY.md` (arc 2+) is the acceptance oracle for htop/nvtop/astral-watch. |
| astral-watch | **Library dependency**, git-pinned to `dce7eee`, `default-features = false`; source auto-selects Prometheus exporter → direct i2c → CSV tail | Reuses `i2c::read_reading`, `alert::evaluate`, `Lifecycle`; never its `tui` feature (ratatui 0.29). Three small upstream PRs make it cleaner (see open question 9). |
| GPU | **nvml-wrapper 0.12.1** on its own thread, fast (250 ms) / slow (1 s) tiers, PCIe from byte-counter fields, `nvidia-smi` only as fallback | dlopen at runtime — no build dep — so the astral-watch "keep NVML soft" reasoning still holds. |
| Audio | **Supervised `pw-record` subprocess** (`--latency 1024`, capture-sink, passive) → SPSC ring → DSP thread (dual FFT 8192/2048, 64 log bands) | Zero dev headers needed (none of libasound/pipewire/pulse -dev are installed); verified command line captures 48 kHz f32 stereo on torch. |
| Media | **zbus 5** hand-rolled MPRIS proxies on one `current_thread` tokio inside `sources`; album art via in-tree half-block painter | The `mpris` crate links libdbus (absent). Firefox is the live player today. |
| Config | Two files: `config.toml` (behaviour, singleton `[sources.*]`, `[[components]]`, `[[rules]]`) and `layout.toml` (pages/placements — the only file edit mode writes) + `themes/*.toml`; 1 Hz mtime hot reload; `toml_edit` comment-preserving saves | Edit mode never rewrites your hand-edited behaviour file. |
| Process tables | **Top-N (default 10, never < 5) inside the 6x3 tiles**, decluttered for the grid — htop `PID RES SHR S CPU% MEM% TIME+ Command`, nvtop `PID DEV TYPE GPU GPU MEM CPU HOST MEM Command` — at the tools' printed widths and formatting (htop's KiB regimes, state colours, auto-width CPU%); `USER`, `VIRT`, `PRI`, `NI`, `ENC`, `DEC` come back through `columns` and in the zoomed `full` tables; one shared pid-level `/proc` scan; a 4x2 shows 7 rows, the 120×40 laptop layout 5; zoom (`z`) fills the body and, from arc 8, gives the full sortable tables with kill/signal | One mechanism, two column sets (`ARCHITECTURE.md` §8.1), verified against htop 3.4.1 / nvtop 3.2.0 sources; per-process GPU accounting is only polled while such a tile is visible. |
| Performance | **21 measured ceilings** (`PERFORMANCE.md`): ≤ 2 % of one core on the Overview beside a game, ≤ 6 % with the visualizer, ≤ 40 wake-ups/s, ≤ 25 KB/s written when static, ≤ 6 ms/s of NVML time (sum listed), no measurable load on Ptyxis (a GPU client itself), RSS ≤ 60 MB, unfocused throttle; each derived from measured costs, not asserted. Showcase-class themes (`matrix`) get their own S-ceilings (≤ 15 % of a core, ≤ 3 MB/s, Ptyxis ≤ +15 % / ≤ 5 % SM) that apply only while focused | Gates per arc, measured unprivileged with per-thread `pidstat`, task-summed context switches, `/proc/<pid>/io`, `nvidia-smi pmon` and `pw-top` on torch (`perf`/`strace` are blocked here); a red cell blocks the commit. |
| Testing | `--demo` (seeded `Synth`), JSONL journal record/replay with a determinism test, insta snapshots of **styled** cell dumps at the real grid sizes, no-panic sweeps, hardware-gated `#[ignore]` tests | CI never needs the GPU; README screenshots regenerate headlessly from arc 2. |

Full detail: `ARCHITECTURE.md` (types, traits, data flow, tick rates, component catalogue, config examples, UX keys, degraded modes, perf budget), `WORKSPACE.md` (tree + pinned deps), `DECISIONS.md` (40 decisions with rationale).

## Review notes from the PE chair

Things I would say out loud before you approve this:

1. **Arc 1 is still two sessions of work, not one.** The critics forced it down by half and it still ships six crates, the store, the layout engine, a three-theme loader, the cpu source, four htop tiers, the app loop and the testkit. Plan it as **1a** (workspace + store + ui core + app loop + `clock` + `modern`/`mono`, `--demo`) and **1b** (cpu source + htop tiers + `retrowave` + testkit + HUD + perf doc). Review and commit after each half. `ROADMAP.md` keeps the single-arc acceptance; treat the split as scheduling, not scope.
2. **The 250×70 terminal size is an assumption**, not a measurement — nothing in the planning session could see your Ptyxis window. The `F12` HUD ships in arc 1 precisely so the grid thresholds get checked against reality on day one; if your usual window is smaller, the `dense`-mode numbers (120×40 → 1x1 = 9×5) are the ones that matter.
3. **Sixel.** The research concluded Ptyxis/VTE 0.84 has no Sixel and chose half-block album art. `libvte` on this box *does* contain Sixel symbols; whether Ptyxis enables it is recorded in `MACHINE.md`. Either way half-blocks stay the default (no `libchafa`, no tty race) — Sixel would be a later opt-in, not a design change.
4. **Six crates is a bet on review, not on ceremony.** If after arc 1 it feels heavy, collapse `components` into `ui` — but keep the two rules that matter: crossterm never below `app`, and components never do I/O.
5. **astral-watch needs a `v0.8.0` tag before arc 3** or gridwatch carries its `clap`/`ureq`/`signal-hook` tree for nothing. The three PRs are small (feature-gate `cli`/`notify`, swap `eprintln!` for the `log` facade, add `Lifecycle::active()`); your crate, your call.
6. **Performance is specified, not hoped for.** `PERFORMANCE.md` names 21 ceilings for four consumers — the gridwatch process, Ptyxis (a GPU client itself: every byte we write is compositor work beside the game), the NVML driver we poll, and the i2c/PipeWire buses we share — each with the tool that measures it and the arc that must pass it. Three cheap mechanisms were added for them: zero-poll, phase-aligned timers (one wake-up serves several sources), a render cache (only the tile whose data changed re-renders), and an unfocused throttle (when the terminal loses focus, animation drops to 2 fps and sources go `Hidden`). An adversarial pass then re-derived every ceiling from measured costs and fixed the ones the design could not meet (D26).
7. **A game is often running on this machine.** The performance headline (<2 % of one core on the Overview page beside a game; <5 % with the visualizer at 30 fps) is a design constraint, not a nice-to-have — it is why demand levels, generation-gated redraws and the pins-only `always_on` exist.

## Decisions resolved (2026-08-31, D35)

1. **Name:** `gridwatch` — repo `github.com/mbeaman/gridwatch`, crate/binary `gridwatch`, workspace crates `gridwatch-*` (was free on crates.io and GitHub when checked 2026-08-30).
2. **Arc 1 scope:** showcase cut — retrowave and the 32-core CCD tier stay in v0.1.0, planned as sessions 1a/1b.
3. **Grid:** 12 × 6 default; pages may override.
4. **Config:** two files (`config.toml` + `layout.toml`).
5. **Mouse:** capture on by default; Shift-drag keeps native selection; `--no-mouse` to opt out.
6. **Audio:** visualizer 30 fps and `pw-record --latency 1024` by default, with `[sources.audio] fps` (up to 60) and `low_latency` as explicit config knobs — the knob is a requirement, not an accident.
7. **Process actions:** kill / renice / affinity / ioprio ship in arc 8, each behind a confirm line and the global `readonly` flag.
8. **Network probes:** ICMP to gateway / 1.1.1.1 / 8.8.8.8 at 1 Hz on; public-IP lookup and reverse DNS off by default.
9. **astral-watch upstream:** yes — cut v0.8.0 and take the three PRs (feature-gate `cli`/`notify`, `log` facade, `Lifecycle::active()`) in an astral-watch session before arc 3.
10. **RAPL:** ship the optional udev rule (`packaging/udev/90-gridwatch-rapl.rules`), documented, never installed by default.
11. **Winamp:** classic skin with custom chrome.
12. **Matrix light:** **deferred** — the current defaults (`fade_s 12`, `trail_ms 900`, `sweep_s 20`, density 0.20, black floor) stand as a starting point, and the theme-vs-usability balance is expected to iterate during arc 4b (Matt, 2026-08-31: "matrix mode still needs a lot of improvement to balance rendering the theme and usability of the modules").

## The arcs (one line each; detail in `ROADMAP.md`)

| Arc | Version | Ships |
|---|---|---|
| 1 | v0.1.0 | Workspace, store, ui core, 12×6 grid engine, theme loader v1 (`modern`/`retrowave`/`mono`), cpu source, htop tiers tiny→cores, `clock`, app loop, `--demo`, testkit, `F12` HUD, measured perf doc |
| 2 | v0.2.0 | GPU source + component with nvtop's rolling charts **and top-N GPU process table**, htop's top-N process table, journal record/replay + determinism test, SVG screenshots in CI |
| 3 | v0.3.0 | `pins` (astral-watch parity) + cross-page alarm overlay, capability probe / `doctor`, staleness, hot reload, theme loader v2, `terminal` + `phosphor-green` |
| 4 | v0.4.0 | Edit mode (move/resize/swap/add/remove/undo, `toml_edit` save), tachyonfx effects, flourishes, `phosphor-amber`, and the **`matrix`** showcase theme (ambient rain, veil, decode, governor) — likely two sessions (4a/4b) |
| 5 | v0.5.0 | Audio visualizer (pw-record → DSP → tiers vu→spectrum) and `sensors` |
| 6 | v0.6.0 | Winamp tile over MPRIS (marquee, big digits, 19-band vis, art, transport) |
| 7 | v0.7.0 | Network component (rates, links, connections, probes) and generic `[[rules]]` alerts |
| 8 | v0.8.0 | Full interactive parity: the zoom-only `full` tiers — htop screens/tree/search/threads/actions, nvtop's every-criterion sort and signal menu — pins CSV/multi-card, `exec` component |
| 9 | v0.9.0 | Packaging (tarballs, deb/rpm, AUR, Nix), theme import, docs, benches |

## How we build: spec-driven at the seams (D33)

An arc starts by updating the spec it implements — `ARCHITECTURE.md` sections, `KEYS.md`, `schema/*.json`, `PARITY.md` rows — and writing acceptance criteria and performance gates in `ROADMAP.md`, before code. Specs are executable where possible (schemas validate fixtures in CI, `gridwatch keys` fails on catalogue drift, view-tree and renderer snapshots pin behaviour); the adversarial review checks the implementation against the spec; a spec change is a `DECISIONS.md` entry. Internals are not spec'd — contracts are binding, private code is free.

## Who builds what (D36)

Fable 5 builds the foundation — arc 1a's core seam, the schemas and testkit, the per-arc briefs, the review gates, the hard kernels; Opus 5 executes the implementation verticals against those briefs, one brief per session, escalating any seam question back to a Fable session. `docs/MODELS.md` is the contract; `docs/REVIEW.md` holds the gate templates; `docs/briefs/arc-1a.md`, `arc-1b.md` and `arc-2.md` are written.

## How a session starts

`CLAUDE.md` is the working agreement (arcs; implement → adversarial review → fix → report → you say commit; commit *before* any shell-enabled review; read-only guard in every agent prompt). Arc 1a (Fable) and arc 1b (Opus) are both built and reviewed. Next session: **arc 2** — write `docs/briefs/arc-2.md` in a Fable session first (D36's spec-first ritual), then implement it. Open against Matt: the real `stty size`, a perf re-take beside the game, the Ptyxis glyph check and the README PNG.

## Map of `docs/`

- `PLAN.md` — this file: the call, review notes, your decisions, the arc table.
- `ARCHITECTURE.md` — the design in full (revision 2 after adversarial review; §16 lists every resolved finding).
- `WORKSPACE.md` — directory tree, crate boundaries, pinned workspace dependencies and the crates deliberately *not* used.
- `ROADMAP.md` — arcs with deliverables, acceptance criteria and risks.
- `DECISIONS.md` — decision log (D01–D40) with rationale; append, never rewrite.
- `PERFORMANCE.md` — 21 performance ceilings, the mechanisms that pay for them, the measurement protocol and per-arc gates.
- `BACKLOG.md` — pre-flight steps, unscheduled wants, and recorded won't-dos; the only place deferred work lives.
- `MODELS.md` — the Fable/Opus division of labor and handoff protocol.
- `REVIEW.md` — the canonical review-gate templates and per-arc checklist.
- `TESTING.md` — the testing contract: five layers, why the harness has a user-facing axis (D46).
- `briefs/` — decision-complete implementation briefs, one per arc half (1a, 1b written).
- `MACHINE.md` — verified inventory of torch: hardware, OS, terminal, audio, permissions, toolchain, what is missing.
- `research/` — ten research digests with verified facts, exact API names and command lines (ratatui 0.30, audio/FFT, MPRIS, htop, nvtop, network, astral-watch + sensors, themes, grid engine, prior art).
- `design-review/` — provenance: three competing proposals, three judge verdicts, two adversarial critiques.
