> **Status: working practice (D36).** The canonical quality gates every arc runs, regardless of which model runs them. Codifies the rules learned on astral-watch: commit before review, read-only guard in every agent prompt, reproduce findings before fixing.

# Review gates

## Invariants (every workflow, every model)

1. **Commit or snapshot first.** `git stash create` prints a commit and stores no ref, and a bare `git tag` tags HEAD — so capture it explicitly:
   ```sh
   git add -A; snap=$(git stash create); git tag review-snap-$(date +%F) "${snap:-HEAD}"   # empty output = clean tree = tag HEAD
   ```
   before any shell-enabled agent sees the tree; verify `git diff --quiet review-snap-<date>` afterwards.
0. **Tooling once per machine:** `cargo install cargo-deny cargo-audit cargo-insta` and `rustup toolchain install 1.88.0` for the local MSRV gate (torch has 1.95 only — MACHINE.md; if rustup is absent, MSRV stays a CI-only check and the report says so).
2. **The guard**, verbatim, in every agent prompt — research, design and review alike:
   > STRICT RULES: read-only. DO NOT create, modify or delete any file anywhere; no git mutation of any kind. Cheap read-only commands and WebFetch/WebSearch are fine. Do NOT run heavy CPU/GPU workloads (a game may be running). Do NOT open /dev/i2c-*.
3. **Reproduce before fixing.** An agent finding is a claim; run the failing case yourself before changing code. Fix confirmed findings; record rejected ones with the reason in the arc report.
4. **Verify against the spec.** Findings cite `ARCHITECTURE.md` / the brief / a schema / `PERFORMANCE.md`; drift can be in either direction — sometimes the fix is a `DECISIONS.md` entry, not code.

## Template A — arc review (after implementation, before commit)

Workflow shape: N finder lenses in parallel → per-finding adversarial verify (refute by default) → synthesis. Lenses for gridwatch (pick 4–6 per arc):

- **correctness** — logic, edge cases, error paths against the brief's task list
- **concurrency** — channels, atomics, the single-writer store rule, no locks on the render thread, panic containment
- **spec-drift** — implementation vs `ARCHITECTURE.md`/schemas/brief, both directions; catalogue vs `KEYS.md` (exists from arc 2)
- **perf-budget** — the arc's `PERFORMANCE.md` gates: cadences, wake-ups, NVML call classes, scan cost, bytes written
- **parity** — the arc's `PARITY.md` rows against the real tool's behaviour (htop/nvtop side-by-side where possible)
- **ux-theme** — tiers at real sizes, theme roles only (no literals), readability pins, degraded modes. `shot` is a supplement here, never the only instrument (D46)
- **user-path** (**mandatory every arc, D46**) — launch the built binary in a pty (`script -qfec "stty rows R cols C; gridwatch run" file`) and try to break it: no tty, one row, 40 columns, resize mid-run, `q`, a killed source, a corrupt `config.toml`. Report *what the user saw* on screen and on stderr — a finding that cites a buffer instead of a terminal is not a user-path finding

Verify stage per finding: "Adversarially verify: <finding>. Try to REFUTE it; default to refuted when uncertain; state exactly how you checked." Keep findings that survive ≥2 of 3 verifiers (or 1 of 1 for cheap arcs), then reproduce by hand.

## Template B — design review (before implementing a seam change)

3 independent proposals from assigned angles → judge lenses (maintainability / performance / product) → synthesis → 1–2 adversarial critics → revision. Used for anything touching a seam; the output lands in `ARCHITECTURE.md` + `DECISIONS.md` before code.

## Template C — pre-cut integration review (before tagging a version)

One agent, whole-diff scope since the last tag: seams coherent, CHANGELOG complete, docs current, no stray scope. Cheap; run it every cut.

## Gate checklist per arc (the report quotes each)

```
scripts/gate.sh          # all of the below in one command, mirroring ci.yml
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
MSRV check (1.88) · per-crate check (store, ui) · feature matrix · cargo deny · cargo tree -d
scripts/perf/measure.sh rows green per PERFORMANCE.md gates table
PARITY.md rows for the arc ticked (the arc that first needs a section creates it — htop lands in 1b)
snapshots inspected via `cargo insta pending-snapshots` + individual diff review and accepted one by one — never a bulk accept
cargo test -p gridwatch --test pty   # D46: the binary under a real pty; its transcript goes in the arc report
```

**Never a passing assertion on its own (D46):** "didn't panic". Every sweep and lattice asserts content — non-blank, the tier's signature, no fabricated readings — and the app-level lint asserts that a size which cannot be drawn *says so*. `docs/TESTING.md` is the contract.
