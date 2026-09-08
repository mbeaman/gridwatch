# Arc 13 brief — "the config means what it says" (D63)

> Written by the Fable session that wrote D63 (2026-09-07). Read D63 first — it decides everything below, including the three keys the backlog did not know were dead, and this brief only says where each piece goes. The two seams are **landed before 13a starts** (table below); if anything here disagrees with `ARCHITECTURE.md` §4.2/§4.3/§9, stop and escalate. **Never change a seam.** Build **13a, then 13b**, sequentially — both halves touch `crates/app/src/app.rs` and `lib.rs`.

## What already exists (do not rebuild)

| seam | where | state |
|---|---|---|
| `OptionIssue { key, kind, text }`, `IssueKind { Rejected, Adjusted }` | `store/src/source.rs`, re-exported from `store/src/lib.rs` | landed |
| `SourceDef.check: fn(&toml::Table) -> Vec<OptionIssue>` | `store/src/source.rs`; registered as a no-op for all seven sources in `sources/src/registry.rs` until 13a replaces each | landed (placeholder) |
| `Retention::for_history(Duration) -> Retention` | `store/src/series.rs`; a test pins `for_history(600 s) == Retention::default()` field for field | landed |
| `Store::retention()`, `Store::footprint() -> Footprint` | `store/src/store.rs` | landed |
| `app::check_sources` (names), its toast at `run`, the reload re-check | `app/src/app.rs`, `lib.rs` (D61) | done — 13a adds the value pass |

## 13a — a source's option values are checked by the code that reads them

**Seam 1 — the reader.** New `crates/sources/src/options.rs`, `pub struct Reader<'a>` over a `&toml::Table` and a source id. Every method returns `Option<T>` (`None` = absent *or* rejected — the caller keeps its default either way) and takes the default so the message can name it: `int`, `int_in(range, unit)`, `int_min(floor)`, `float`, `bool`, `str`, `one_of(&[&str])`, `str_list(allow_empty)`, `int_or_str` (audio `sink`), `bool_or_words` (sensors `rapl`), `adjusted(key, text)` for a caller-composed note, plus `finish() -> Vec<OptionIssue>` and `asked() -> &[&'static str]`.

Message shapes, verbatim (the `sources.<id>: ` prefix is added by `check_sources`, not the reader; `toml::Value::type_str()` supplies the type word):

- Rejected type: `` `refresh_ms` expects an integer (milliseconds), found a string ("1500") — the default 1500 stands ``
- Rejected floor: `` `device` expects an integer >= 0, found -1 — the default 0 stands ``
- Rejected choice: `` `source` expects one of auto, i2c, exporter, found "i2x" — auto stands ``
- Rejected list: `` `probes` expects a list of strings, found 5 at [1] — the default ["gateway", "1.1.1.1"] stands ``; empty: `` `chips` is an empty list — the default ["*"] stands ``
- Adjusted clamp: `` `interval_ms` = 100 clamped to 500 (accepts 500-5000, P14) `` — keep the parenthetical each source's warning carries today.

**Seam 2 — seven `from_table`s return `(Options, Vec<OptionIssue>)`**, each ending in `r.finish()`, with `pub fn check(t: &toml::Table) -> Vec<OptionIssue> { Options::from_table(t).1 }` beside it, registered in `registry.rs` in place of the placeholder. `start` logs each issue once at `warn!` and the scattered `tracing::warn!`s go. **cpu and gpu gain an `Options` type** (`cpu::Options { refresh, k10temp }`, `gpu::Options { refresh, device }`); `cadence_from` takes it. The per-key contract — kind, range, default, and what the code does silently today — is the table in D63's brief section; **do not narrow an accepted shape**: `sink` is int-or-string, `rapl` bool-or-words, every float accepts an integer.

**Seam 3 — `check_sources`' value pass** (`app/src/app.rs`). `SourceReport` gains `pub warnings: Vec<String>`. For a known source, after the name loop, `for issue in (def.check)(table)`: `Rejected` pushes a `lines` entry and a `failures` entry; `Adjusted` pushes a `lines` entry prefixed `warning:` and a `warnings` entry. Do not echo the `{id} — {key} = {value}` line for a key that has an issue. **Delete** the `(option names are checked here; values are the source's own)` line added by the arc-11 review and the doc comment's claim above it. `Shell.source_warnings` carries failures then warnings; the toast loop, `attach_plugins` and the reload need no shape change beyond reading both. Only `failures` reach the exit code.

**Tests for 13a.** `options.rs`: one per method, asserting the exact `text` and the `kind`. Each source's options test becomes: every option x wrong type => `Rejected` naming the real default; out of range => `Adjusted` with the clamped value; the choice and word sets; and **the tripwire** `Reader::asked() == OPTION_NAMES` on an empty table, replacing the `OPTION_NAMES.len() == N` assertions. `app/tests/shell.rs` beside the D61 cases: a wrongly typed value fails and says what stands; a clamped value warns and never fails; the shipped default yields neither. Pty **C.34** beside C.33: the string value => exit 1 and the sentence; the clamp => exit 0 with `warning:`; `--demo` with the string value => the toast, then a clean `q`.

## 13b — every key the config accepts is read, or says it is not

**Seam 4 — `[store] history` is live.** `app/src/config.rs`: `parse_history(&str) -> Result<Duration, String>` accepting `<n>s`, `<n>m`, `<n>h` and compounds (`1h30m`), integer arithmetic only. `load_from` parses it and returns a load error naming the file on failure (the shape of `unknown borders mode`), clamps to `1m..=1h` with a warning, and `Loaded` gains `pub retention: Retention` from `Retention::for_history`. `Shell::new` builds `Store::new(loaded.retention)`. The reload path toasts `"[store] history changed — the store is sized at start; restart to apply"` when the resolved retention differs. `config check` prints the resolved numbers.

**Seam 5 — three keys retire, one section tells the truth, loader warnings reach the screen.** `StoreSect.max_mb`, `PerfSect.phase_ms` and `ConfigFile.confirm_kill` become `Option`s; each `Some` pushes a warning naming what nothing read and telling the reader to delete the line. Remove the three from `defaults/config.toml` (the structural-default test must still pass) and mark them `deprecated` in `schema/config.schema.json`, where `history` also gains a duration pattern. `[record]`'s stale "arrives in arc 2" warning becomes the truth about `--record FILE`. In `lib.rs`, toast every `loaded.warnings` line at start — today they are only logged.

**Seam 6 — the footprint on screen.** The `F12` HUD gains one line from `Store::footprint()`, computed only while the HUD is up; `--stats-log` gains `store_bytes` at its 1 Hz tick. Never per frame.

**Seam 7 — the config audit test.** `app/tests/shell.rs`: parse `DEFAULT_CONFIG`, walk its leaf keys (skipping `sources.*`, `components`, `rules`, `plugins`, which have their own checks), and require each in a table of `(path, mutated config, assertion)`. The failure message names the path and says "say what consumes it, or retire it (D63)". Plus the three-way pin: `Retention::default() == for_history(parse_history("10m"))` and `== Loaded::from(DEFAULT_CONFIG).retention`.

**Tests for 13b.** `parse_history` accepts and rejects the listed forms; the clamp warns both ways; each retired key present produces its warning; `[record]` produces the new wording. A reload with a different `history` toasts "restart to apply" and leaves `retention()` unchanged. Pty **C.35**: `history = "1h"` => the resolved numbers and exit 0; `history = "ten"` => exit 1 naming `config.toml`; a 2026-09 config carrying `max_mb = 32` => exit 0 with the retired warning. `replaying_a_fixture_twice_is_byte_identical` unchanged and green — it is the determinism gate.

**Docs for 13b.** The D63 ARCHITECTURE edits are already applied; 13b adds `PERFORMANCE.md`'s P5 rows (reworded to the mechanism that exists — cadence alignment, not a phase grid) and P17, `CHANGELOG.md`, `PLAN.md`, and `BACKLOG.md` (both `P1`s struck as pulled, the `Ring::new` preallocation item added).

## Numbers

**P17 at the ceiling**: a 60-minute release run under a pty at 250x70 on an idle torch with `history = "1h"` and `--stats-log` — RSS at 1/20/40/60 min and `store_bytes` at the end, beside the arc-3b/5a rows. A second 20-minute run at the default confirms `store_bytes` under 6 MB. **P18**: state, do not re-measure — `for_history(600 s)` is field-identical to `Retention::default()`, so the sweep numbers stand.

## Traps

1. **Determinism is config, not clock** — every replay and `shot` builds from `load_embedded`, which keeps `10m`; `parse_history` and `for_history` are integer arithmetic; the three-way pin is the gate.
2. **`check` is pure.** No `std::fs`, no `Command`, no device probe, in any `from_table`.
3. **Names are D61's, values are D63's**: the reader never reports a key it was not asked for; the tripwire proves the asked set equals `OPTION_NAMES`.
4. **Do not narrow an accepted shape** — a config that ran yesterday fails today only where yesterday it was silently discarded.
5. **The pty counts are exact**: the shipped default must produce zero issues or C.33 breaks.
6. **Toast shape** (D61 R2): head `sources.<id>: \`key\``, tail `— … stands` / `clamped to …`; the found value is the middle that may go.
7. **`k10temp`'s default is the build's**: print it, do not hard-code.
8. **Store time moves only with batches**: under pause only `always_on` pins advance it, so a short `history` evicts every `net`/`sensor` label after `max_age` of pause — say so in the CHANGELOG, do not "fix" it.
9. **`Ring::new` preallocates `min(max_len, 4096)`**: `history = "1h"` costs 64 KB a series before a point lands; that is P17's job to show, not this arc's to change.
10. **Retired fields are `Option`s for one version**, not deleted: every existing install's config carries them.

## Gates

`scripts/gate.sh` green; commit before review; the review workflow from `docs/REVIEW.md` with the read-only guard in every agent prompt, and a lens that greps every `check` for I/O; scoped commits (`sources:`, `app:`, `store:`, `docs:`), 13a's before 13b's so each half reverts alone; push `main` and watch CI. `v0.13.0` and the `Cargo.toml` version are Matt's.
