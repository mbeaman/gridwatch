# gridwatch — working agreement

Modular, themeable ops-dashboard TUI for Matt's workstation "torch", in Rust (ratatui 0.30). **Status: arc 1 is built, committed and pushed (2026-09-01); CI green. `v0.1.0` is untagged pending the four rows only Matt can close (P4/P21 in Ptyxis, the glyph check, the README PNG). Next: arc 2 from `docs/briefs/arc-2.md` (D47): session 2a (journal, pid scan, htop table, tooling) then 2b (GPU).** Name: **`gridwatch`** (crates `gridwatch-*`). The on-disk directory remains `~/workspace/opsTui`; don't rename it (Claude memory is keyed to the path — migrate `~/.claude/projects/-home-mattbeam-workspace-opsTui/memory/` first if it ever moves).

## Model strategy (D36 — details in `docs/MODELS.md`)

- **Fable 5**: seams and foundation (arc 1a core, schemas, testkit, measure.sh), per-arc implementation briefs (`docs/briefs/`), review-gate judging, gnarly kernels (DSP math, matrix compositor, i2c/Lifecycle edges, NVML quirks), any seam change (+ DECISIONS entry).
- **Opus 5**: implementation verticals against a brief — one brief ≈ one session; also fixtures, doc regeneration, applying confirmed review findings.
- **Any model, always**: work from the arc's brief; never change a seam — escalate instead; end the arc with the `docs/REVIEW.md` gates and the `PERFORMANCE.md` rows; commit-before-review; read-only guard in every agent prompt.

## Start of every session

1. Read `docs/PLAN.md`, then the current arc in `docs/ROADMAP.md` **and its brief in `docs/briefs/` if one exists**, then the tail of `docs/DECISIONS.md` and `git log --oneline -20`. Deferred work lives only in `docs/BACKLOG.md` — pulling an item into an arc is a DECISIONS entry.
2. Give a short state read and offer arc options with a recommended next step. Wait for Matt to pick.

## How work happens

- **Arcs.** One arc ≈ one coherent phase ≈ one minor version (`v0.N.0`), with a `CHANGELOG.md` entry. Arc 1 is planned as two sessions (1a/1b).
- **Per arc:** implement → adversarial review (Workflow/agents) → fix confirmed findings (reproduce them yourself first) → report → **Matt says commit / push / tag.** Never commit, push, tag or publish unprompted.
- **Commit before review.** Snapshot (`git stash create` → tag) or commit before handing uncommitted work to any shell-enabled agent, and verify the tree is byte-identical afterwards.
- **Read-only guard in every agent prompt** — research, design and review alike: "do not create/modify/delete files, no git mutation". A design workflow once wrote files into a repo; a review agent once ran `git restore` over uncommitted work.
- **Commit messages:** `area: imperative summary` (e.g. `layout: derive dense threshold from GridSpec`), one scoped area per commit; split risky halves so they revert alone.
- **Gate before any commit:** `scripts/gate.sh` (one command, mirrors `ci.yml`; `--quick` for the inner loop) — i.e. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace` (which includes the pty suite — `crates/cli/tests/pty.rs` runs the binary under a real terminal, D46), `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings`, MSRV **1.88** check. CI mirrors astral-watch's (`dtolnay/rust-toolchain`, `Swatinem/rust-cache`).

## Architecture rules that reviews enforce (see `docs/ARCHITECTURE.md`)

- Crate direction `store ← ui ← components ← app ← bin` and `store ← sources ← app`. **crossterm only in `app`/`bin`.** Components depend on `store` + `ui`, never on `sources`.
- Components **describe, never do**: `view(&self, cx) -> View` returns a semantic tree; the theme's renderer draws it. No I/O, no threads, no device access, no `Instant::now()` (use `cx.now`), no colour/glyph literals, no cell writes outside `View::Custom` (and there only through theme roles). Side effects are returned as `Command`s.
- The render thread owns the `Store`; the only mutation is `Store::apply(&Msg)`. Sources are singletons per kind, configured under `[sources.<id>]` only; instance options are view-only.
- Every component's `tiers()[0].min` fits **8×3**; tiers are cumulative, poorest first. Test with `assert_min_tier_fits` and `assert_renders_everywhere` — "didn't panic" is never a passing assertion on its own (D46, `docs/TESTING.md`); components declare `signature(tier)`.
- Never call `ratatui::init` / `try_init` / `run` / `restore` (`clippy.toml` bans them): the app owns raw mode, alternate screen, mouse capture and the panic hook.
- No `unsafe`. No `.lock().unwrap()` on the render thread. Std threads by default; tokio only inside `sources` behind `mpris` / `net-*` features.
- Performance ceilings in `docs/PERFORMANCE.md` are commit gates (P-class for quiet themes, S-class for showcase themes while focused).
- `deny.toml` bans `cpal`, `mpris`, `pipewire`, `libpulse*`, `ansi_colours`, `enum-map`, `sysinfo` — each has a documented reason in `docs/WORKSPACE.md`. Don't re-add them.
- Two config files: `config.toml` (behaviour) and `layout.toml` (pages/placements — the only file edit mode writes). Themes in `themes/*.toml`; a theme's `class` (`quiet`/`showcase`) decides which performance ceilings apply. Ambient effects never veil the focused tile, an alerting tile, the banner or the key bar, and always freeze on `FocusLost`.

## Spec-driven at the seams (D33)

- The spec = `docs/ARCHITECTURE.md` (contracts with signatures) + `docs/KEYS.md` (generated) + `schema/*.json` + `docs/PARITY.md` + `docs/PERFORMANCE.md`. **Update the spec before the code** for every arc; write acceptance criteria and gates in `ROADMAP.md` first.
- Executable specs: schemas validate fixtures in CI; `gridwatch keys` must not drift; view-tree + renderer snapshots pin behaviour; PARITY rows are ticked by tests or by hand with a note.
- Reviews verify implementation against the spec and flag drift both ways; a spec change is a `DECISIONS.md` entry. Internals are free — only seams are binding.

## Machine notes (details in `docs/MACHINE.md`)

- A game is frequently running: don't launch heavy CPU/GPU jobs; performance budgets assume that neighbour.
- Agents must not open `/dev/i2c-*` (astral-watch's chip is on the GPU's bus). Matt is in the `i2c` group; run `astral-watch`/pins tests by hand.
- `pw-record`, `pw-dump`, `wpctl`, `nvidia-smi`, `busctl --user` are safe read-only probes.
- No Nerd Fonts installed: the `unicode` glyph tier (box drawing, blocks, braille; VTE draws octants natively) is the default.
- Sibling checkouts: `../astral-watch` (Matt's, git-pinned dependency; `[patch]` it via a git-ignored `.cargo/config.toml`), `../gpuwatch` (third-party C++ NVML reference, read-only).

## Where things live

`docs/PLAN.md` (entry point) · `ARCHITECTURE.md` · `WORKSPACE.md` · `ROADMAP.md` · `DECISIONS.md` · `TESTING.md` · `MACHINE.md` · `research/` (verified digests) · `design-review/` (why this design won).
