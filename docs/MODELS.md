> **Status: working practice, adopted 2026-08-31 (D36).** How to split gridwatch work between Claude Fable 5 (`claude-fable-5`) and Claude Opus 5 (`claude-opus-5`). Switch with `/model`; fast mode is Opus.

# Model strategy — Fable builds the foundation, Opus builds the verticals

The rule of thumb: **give Fable the work whose mistakes tax every later arc; give Opus the work whose mistakes a test catches.** The plan was deliberately shaped so that most code falls in the second bucket: seams are spec'd with real signatures, formats have schemas, behaviour is pinned by snapshots and fixtures, and each arc has acceptance criteria and performance gates. That scaffolding is what lets Opus sessions run fast without eroding quality — and building it *is* Fable's job.

## Fable owns

1. **Arc 1a — the core seam, implemented by Fable.** The six-crate scaffold, `gridwatch-store` (Ts/Clock, Key/Datum/Record catalogue, Store::apply/resample, the three channels, Source/Demand/Detail, `demo::Synth`), the `gridwatch-ui` contracts (Component/View/Tier/Manifest/Registry, the layout solver with its proptests, theme loader v1, default Renderer), the app loop (render cache, coalescing, zero-poll timers, terminal lifecycle + PanicPolicy, input mirror), the testkit, `clippy.toml`/`deny.toml`/CI. Every vertical inherits this code's correctness; it is the single highest-leverage block in the project.
2. **Executable specs.** `schema/*.json` + fixture validation, the journal determinism harness (arc 2 core), `scripts/perf/measure.sh`, golden fixtures' *shape* (Opus can record more later).
3. **Per-arc implementation briefs** (`docs/briefs/arc-N*.md`), written at arc start per the D33 spec-first ritual: decision-complete build order, the verified API names and gotchas distilled from `docs/research/`, the test list, the acceptance mapping. A brief is done when an implementer needs no other document open except `ARCHITECTURE.md`.
4. **The review gates.** Designing and judging the adversarial review workflows (`docs/REVIEW.md` holds the canonical templates), verifying implementations against the spec, signing off performance gates, reproducing findings before fixes.
5. **The gnarly kernels**, pre-shaped or post-reviewed even when they sit inside an Opus vertical: the DSP binning/ballistics math (arc 5), the matrix sole-compositor and its governor (arc 4b), the i2c contention and Lifecycle bridge edge cases (arc 3), NVML field-quirk handling (arc 2). Fable writes the signature + the tests, or reviews the result adversarially.
6. **Seam changes.** Any change to Component/Source/Store APIs, the theme/layout/config schemas, the wire protocol or the journal format is a Fable task with a `DECISIONS.md` entry.

## Opus owns

- **Implementation verticals against a brief**: arc 1b (htop tiers, widgets, retrowave, HUD), arc 2 (gpu source/component, process tables, journal wiring), arc 3 (pins + alerts + doctor), arc 5 (audio), arc 6 (winamp), arc 4a (edit mode), arc 4b (effects + matrix — with the compositor/governor kernel pre-shaped by Fable per §5 above), arc 7 (net + rules), arc 8's parity breadth (the `exec` plugin host in that arc is a Fable seam), arc 9 (packaging) — one brief ≈ one session.
- **Mechanical breadth**: recording fixtures, regenerating docs (`KEYS.md`, `COMPONENTS.md`), adding cards to the spec table, widening test matrices, applying review findings that Fable confirmed.
- **Rules for an Opus session** (also in `CLAUDE.md`): work from the arc's brief; never change a seam — if the brief is ambiguous or a seam seems wrong, *stop and escalate* rather than improvise; end every arc with the `docs/REVIEW.md` gate and the `PERFORMANCE.md` gates; commit-before-review and the read-only agent guard apply regardless of model.

## Session topology — who sits in the driver's seat (D40)

Not every session runs Fable as an orchestrator with Opus subagents. Three topologies, matched to the work:

1. **Fable main-loop** (seams, briefs, design judging, arc 1a): Fable drives; it *may* fan out subagents for mechanical breadth (searches, fixture recording, doc regeneration) and those can run on cheaper models — but the judgment calls stay in the main loop.
2. **Opus main-loop for a vertical arc** — the default for implementation, and deliberately *not* "Opus as a subagent under Fable". An arc is a sequential, stateful build: compile → test → fix loops over a growing tree, where one agent holding the whole context beats orchestrated task-slices, and a Fable supervisor idling over it would spend the expensive model on babysitting the brief already encodes. Fable's oversight is asynchronous by design: the brief before, the gates after.
   **The hybrid that matters:** the harness lets a session spawn subagents on any model — so an Opus arc session runs its Template-A **verify/refute stage on Fable subagents** (`model: fable` per verify agent, guard included). Opus implements and proposes findings; Fable adversarially verifies them in-session; anything disputed or seam-shaped still escalates to a Fable main-loop session. Finder lenses can run on Opus (or cheaper); spend Fable where refutation quality decides what gets fixed.
3. **Fable-orchestrator + Opus workers** — reserved for wide, independent, spec-complete task sets where fan-out wins: arc 8's parity-checklist breadth, arc 9's packaging matrix, mass fixture generation, corpus-wide sweeps. Fable holds the spec and validates each returned unit.

Rule of thumb: **sequential + stateful → one strong main loop; parallel + independent → orchestrate.** And in every topology the gates, the read-only guard and commit-before-review are model-independent.

## Handoff protocol per arc

```
Fable: update spec sections + write docs/briefs/arc-N.md  →  Matt approves the brief
Opus:  implement the brief (1 session)                     →  run the review workflow (REVIEW.md)
Fable: verify findings on seams / judge anything disputed  →  gates green
Matt:  commit / tag
```

**Escalate from Opus to Fable when:** a seam needs changing; the brief and the spec disagree; a performance gate stays red after one fix attempt; anything touches concurrency primitives, `unsafe` (banned anyway), the panic policy, or the store's apply path; a review finding is disputed rather than confirmed.

## Already banked (Fable, this planning session)

The machine survey (`MACHINE.md`), ten verified research digests with exact API names and measured costs (`docs/research/`), the judged architecture (`docs/design-review/`), the spec itself (`ARCHITECTURE.md`) and the decision log (`DECISIONS.md`, D01–D37), the performance ceilings and protocol (`PERFORMANCE.md`), the review templates (`REVIEW.md`), and the arc 1a/1b briefs (`docs/briefs/`). An Opus session starting arc 1b should need: `CLAUDE.md` → `briefs/arc-1b.md` → the `ARCHITECTURE.md` sections it names, plus `REVIEW.md` and `PERFORMANCE.md` at arc end — nothing else.
