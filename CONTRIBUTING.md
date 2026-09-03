# Contributing

gridwatch is one person's dashboard that got written carefully enough to hand
to someone else. That shapes what a good contribution looks like: the design is
opinionated and written down, and the fastest way to have a patch land is to
read the opinion before arguing with it.

## Before code

```console
$ git clone https://github.com/mbeaman/gridwatch && cd gridwatch
$ cargo run -- --demo          # the whole dashboard on synthetic data, no hardware
$ scripts/gate.sh --quick      # fmt, clippy, tests — the inner loop
```

You need **Rust 1.88+** and nothing else. Every optional dependency is
`dlopen`ed or spawned, so the workspace builds on a machine with no GPU, no
PipeWire and no D-Bus; `--demo` gives every tile data. If a build needs a
system package, that is a bug — say so.

`gridwatch doctor` tells you what this machine can and cannot do, with the fix
for each.

## The one command that matters

```console
$ scripts/gate.sh              # exactly what CI runs
$ scripts/gate.sh --quick      # the fast subset, for the inner loop
```

`gate.sh` is `cargo fmt --check`, `clippy --all-targets --all-features -D
warnings`, `cargo test --workspace`, `cargo doc -D warnings`, the MSRV check,
the per-crate and feature-matrix checks, `cargo deny`, and the generated-doc
drift checks. If it is green, CI is green. Run it before you open anything.

## Where to read first

| you want to | read |
|---|---|
| add a tile | [`docs/ADDING-A-COMPONENT.md`](docs/ADDING-A-COMPONENT.md) |
| add a tile *without* recompiling gridwatch | [`docs/PLUGINS.md`](docs/PLUGINS.md) |
| add a theme | [`docs/THEMES.md`](docs/THEMES.md), or `gridwatch theme import` |
| move tiles around | [`docs/LAYOUT.md`](docs/LAYOUT.md) |
| understand why any of it is shaped this way | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), then [`docs/DECISIONS.md`](docs/DECISIONS.md) |

`DECISIONS.md` is append-only and answers most "why not just…" questions,
usually with the measurement that settled it.

## The rules that reviews actually enforce

These are not style preferences. Each one is load-bearing for something.

- **Crate direction** is `store ← ui ← components ← app ← bin` and
  `store ← sources ← app`. **crossterm appears only in `app` and `bin`.**
- **A component describes; it never does.** `view(&self, cx) -> View` returns a
  semantic tree and the theme's renderer draws it. No I/O, no threads, no
  device access, no `Instant::now()` (use `cx.now`), no colour or glyph
  literals. Side effects leave as `Command`s. This is what makes one render
  path work for seven themes and what makes replay byte-identical.
- **The render thread owns the store.** The only mutation is `Store::apply`.
  No locks on the render thread.
- **Every tier-0 fits 8×3**, and a tier's `signature` is asserted at every size
  that picks it. "It didn't panic" is never a passing assertion
  ([`docs/TESTING.md`](docs/TESTING.md)).
- **No `unsafe`.** There are two documented seams (`app::sys`,
  `app::proc_actions`), each with a `SAFETY:` note per call. A third needs a
  `DECISIONS.md` entry, not an `#[allow]`.
- **Performance ceilings are commit gates.** [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md)
  has 22 of them with the tool that measures each. A new poller with no
  measured row is how a budget stops meaning anything.
- **Generated files are generated.** `docs/KEYS.md`, `docs/KEYBINDINGS.md`,
  `docs/COMPONENTS.md`, `THIRD_PARTY.md` and `docs/img/*.svg` come from
  `scripts/shots.sh` and `scripts/third-party.sh`. Run them, commit the result;
  CI fails on drift.

## Commits and pull requests

- One scoped area per commit, `area: imperative summary` —
  `layout: derive dense threshold from GridSpec`. Split risky halves so they
  can be reverted alone.
- Say **why** in the body, and what you measured if you measured anything. The
  history here is meant to be readable in a year.
- Add a `CHANGELOG.md` entry under `[Unreleased]`.
- A change to a **seam** — a public trait, a message type, a config surface, a
  schema — needs a `DECISIONS.md` entry saying what moved and why. That is the
  project's one piece of ceremony and it exists because seams are what other
  people build against.

## Tests

Five layers, described in [`docs/TESTING.md`](docs/TESTING.md). The two habits
worth copying:

- **Assert the thing that would regress.** A test that only checks "it still
  drew" passes for the wrong reasons; a flooding-plugin test that measured
  frames rather than messages passed before the bug it was written for was
  fixed.
- **Check a regression test both ways.** Break the fix, watch the test go red,
  put it back. It takes a minute and it is the difference between a test and a
  decoration.

Nothing may act on a process it did not spawn — the process-action tests run
against their own `sleep` child behind a pid fence, and that is not negotiable.

## What is out of scope, and why

[`docs/BACKLOG.md`](docs/BACKLOG.md) has a "Won't do" section with the reason
for each, so you can tell "nobody got to it" from "we tried and it does not
work". Wanted-but-unscheduled things are in the same file; pulling one into an
arc is a `DECISIONS.md` entry.

## Reporting something

A good bug report here is the output of:

```console
$ gridwatch --version
$ gridwatch doctor
$ gridwatch config check
```

plus what you expected. If the screen is wrong, `S` writes a screenshot into
`$XDG_STATE_HOME/gridwatch/`, and `--record FILE` writes a journal that replays
the exact data you saw — `gridwatch run --replay FILE` reproduces it frame for
frame on any machine, which is usually faster than describing it.
