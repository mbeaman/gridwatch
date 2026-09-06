# Arc 11 brief — "the seams say so" (D61)

> Written by the Fable session that wrote D61 (2026-09-05). The two contract edits are **done and on `main`**; this brief is the breadth that follows them, for an Opus session. Read `docs/DECISIONS.md` D61 first — it is short, it answers the seven questions the survey (`seam-escalations-survey.md`) posed, and every design choice below is only restated here, not re-decided. **Never change a seam**: if anything here disagrees with `ARCHITECTURE.md` §4.1/§4.2/§4.3/§9, stop and escalate.

## What already exists (do not rebuild)

| seam | where | state |
|---|---|---|
| `SourceDef.options: &'static [&'static str]` | `store/src/source.rs`, filled in `sources/src/registry.rs` from each source's `OPTION_NAMES` | done |
| `LabelSet { Static, Dynamic }`, `KeyMeta.labels`, `key::labels_dynamic(name)` | `store/src/key.rs`; every row in `store/src/keys/*.rs` | done — `net.*` and `sensor.*` labelled keys are `Dynamic` |
| `Retention.max_uncatalogued` (512) | `store/src/series.rs` | done |
| `PerSource.capped`, `Store::capped(id)` | `store/src/store.rs` | done |
| per-label eviction in `Store::sweep`; the cap in `Store::apply` | `store/src/store.rs` | done, each with a store test |

The `ARCHITECTURE.md` lines were updated before the code (D33) and are the contract.

## 11a — the config cannot be quietly wrong about a source

**Seam 1 — `app::check_sources`.** In `crates/app/src/app.rs` beside `check_components` (arc 10a), same report shape (`lines`, `failures`):

```rust
pub(crate) fn check_sources(registry: &Registry, loaded: &Loaded, plugin_ids: &BTreeSet<String>) -> SourceReport
```

For each `(id, table)` in `loaded.config.sources`:
- `registry.source(id)` is `Some(def)`: every key of `table` not in `def.options` is a failure: `` sources.cpu: unknown option `refres_ms` (accepts refresh_ms, k10temp) ``. A key that *is* accepted is listed in `lines` as `  cpu — refresh_ms = 1500` so the check shows what it read (the component pass prints options the same way).
- `None` and `plugin_ids.contains(id)`: one warning line — `  weather — [sources.weather] is not delivered to a plugin under contract 1; ignored`. Not a failure (D61).
- `None` otherwise: a failure — `` sources.cpus: no such source in this build (have audio, cpu, gpu, …) `` — the registered ids sorted, since a feature compiled out and a typo look the same.

`config_check` (`crates/app/src/lib.rs`) calls it after the plugin pass (so `plugin_ids` is filled) and before the component pass; its `failures` join the others and make the exit non-zero. A value's *type* is not checked here — that is the source's own `Options::from_table` and stays there.

**Seam 2 — the toast at `run`, and on reload.** `Shell::new` runs the same function and toasts each failure once (Warn), after `view_warnings`; the source still starts. In the reload path (`app.rs` where `[sources.*] changed — sources are configured at start; restart to apply` is toasted) re-run the check on the new table and toast its failures too — the person is looking.

**Seam 3 — the disjointness test, every pair.** `crates/app/tests/shell.rs::source_and_component_option_names_are_disjoint` becomes a table `[(&str, &[&str], &Manifest)]` of every component's `OPTION_NAMES` and manifest (`htop`, `gpu`, `pins`, `audio`, `sensors`, `winamp`, `net` — the ones that declare `OPTION_NAMES`), checked against `registry.source(id).options` for each id in `manifest.sources` and `manifest.optional_sources`. The assertion message names the component, the source and the key. The table is test code (D61): `Manifest` gains nothing.

**Tests for 11a.** In `shell.rs` (they need the registry): an unknown key fails with the accepted set in the message; an accepted key passes; an unknown id fails with the registered list; a plugin id warns and does not fail; `config check` exits non-zero end to end through the pty suite if a fixture config is cheap to add (`crates/cli/tests/pty.rs` shows how) — otherwise a unit test on `config_check`'s `CheckReport` is enough.

## 11b — the store cannot quietly grow, and says so

**Seam 4 — `KEYS.md`.** `keys_doc` in `crates/app/src/lib.rs` gains a `labels` column after `source`: `static` / `dynamic`, from `meta.labels`. Regenerate `docs/KEYS.md` (`cargo run -- keys > docs/KEYS.md`, or whatever `scripts/` does); CI's drift check must pass.

**Seam 5 — the `sources` tile.** Where the row shows `dropped`, show `capped N` when `store.capped(id) > 0`, in the same style. It is a note, not an alert.

**Seam 6 — the tests the arc owes (see ROADMAP arc 11 for the list).** The two store tests written with the seams cover the happy path of each; the arc adds the ones in the roadmap's "proven from both ends" bullet, in `crates/store/tests/store.rs` next to `a_scalar_published_once_survives_the_sweep`. Use store `Ts`, never the wall clock. The replay determinism test that crosses an evicting sweep can reuse `the_sweep_is_driven_by_store_time_not_the_wall_clock`'s shape.

**Seam 7 — the numbers.** Re-run `the_retention_sweep_stays_inside_the_batch_budget` in release and write the number into `PERFORMANCE.md`'s log beside arc 10b's 2.3 µs; add one for the cap on a new-series insert of an uncatalogued name (a `HashMap` increment — it should be tens of nanoseconds, and the row exists so nobody has to wonder).

## Traps

1. **Do not evict per series.** D61 says why: `sensor.max_c{chip}` and `net.speed_mbps{iface}` are published once for labels that live forever. The sweep's liveness is per `(domain, label)`; a test that only checks a whole label going quiet will pass a per-series implementation, so keep the "once-published scalar beside its live sibling survives" case.
2. **`Label::None` is never removed**, even for a `Dynamic` key, even uncatalogued.
3. **The domain is the name's prefix before the first `.`** — for `sensor.temp_c` it is `sensor`, not `sensors`; the cpu source publishes `sensor.*` k10temp keys when the sensors feature is off (§16) and they share the label vocabulary, which is why the grouping is by name prefix and not by `KeyMeta.source`.
4. **A raised rule pins its label** (arc 10b's `raised_for`); the eviction skips it and the states survive. Nothing in this arc resolves alerts.
5. **Capped samples are refused, not queued**, and `capped` is never reset — the count is the evidence.

## Gates

`scripts/gate.sh` green; the review workflow from `docs/REVIEW.md` with the read-only guard in every agent prompt; commit before review; scoped commits (`app: …`, `store: …`, `docs: …`); push `main` and watch CI. `v0.11.0` and the `Cargo.toml` version are Matt's.
