> **This is a survey, not a brief.** Written by an Opus session on 2026-09-05 —
> `MODELS.md` calls this "mechanical breadth" (searches, spec-table cards), and
> deliberately stops short of the seam decisions. The **brief and the DECISIONS
> entry are the Fable session's**, per D36: both items below change a Rust
> contract, which is why arc 10 escalated them (D60) instead of building them.
> Every claim here was checked against the code and the line numbers are from
> `5cbd8c0`.

# Survey for the two seam escalations

`BACKLOG.md` carries the designs arc 10 wrote when it declined to build these.
This file adds what a session would otherwise spend its first hour
rediscovering — including **two findings that change the shape of the first
item** and one that adds a decision to the second.

## E1 — `[sources.<id>]` options are not validated

### The finding that changes the shape: the truth is already declared

`BACKLOG.md` says the fix is to "add `options: &'static [&'static str]` to
`SourceInfo` … have each source's `Options` type name its keys once". **Each
source already names its keys once.** All seven declare a `pub const
OPTION_NAMES: &[&str]`:

| source | file:line | keys |
|---|---|---|
| cpu | `sources/src/cpu/mod.rs:40` | `refresh_ms`, `k10temp` |
| gpu | `sources/src/gpu/mod.rs:32` | `refresh_ms`, `device` |
| pins | `sources/src/pins/mod.rs:31` | `source`, `exporter`, `interval_ms` |
| audio | `sources/src/audio/mod.rs:34` | 10 keys |
| sensors | `sources/src/sensors/mod.rs:27` | `refresh_ms`, `chips`, `rapl` |
| mpris | `sources/src/mpris/mod.rs:31` | `player`, `art`, `art_max_px`, `poll_ms`, `history` |
| net | `sources/src/net/mod.rs:29` | 7 keys |

So this is **not** a per-source refactor. It is a field on `SourceInfo` (or a
`SourceDef` accessor), seven one-line registrations, and a check. Arc 10's
escalation reasoned "a key table in `app` would be a second copy of the truth"
— correct, and it missed that the *first* copy already exists in the right
place and simply is not reachable from the registry.

### What consumes `OPTION_NAMES` today: almost nothing

Only two things, and both are tripwires rather than uses:

- a per-source `assert_eq!(OPTION_NAMES.len(), N)` unit test (audio, sensors,
  mpris, net have one; cpu, gpu, pins do not);
- `app/tests/shell.rs:502` `source_and_component_option_names_are_disjoint`,
  which enforces §9's disjointness rule **for exactly one pair** — htop against
  cpu. The other six sources and nine components are unchecked. §9 states the
  rule generally; the test does not. Worth folding into the same change, since
  it is the same list becoming reachable.

### Where the wiring goes

- `SourceInfo` — `store/src/source.rs:150`, **four** fields today (`id`,
  `produces`, `cadence`, `requires`). Documented as a signature in `ARCHITECTURE.md:101`;
  `SourceDef` at `:115`.
- Constructed in **`store/src/demo/*.rs`** as `<kind>_info()` — seven sites
  (`net_info` at `demo/net.rs:319` is the pattern). Note the constructors live
  in `demo`, so a field added there is filled in by the crate that owns the
  catalogue, not by `sources`. That is a wrinkle worth a decision: the option
  names live in `gridwatch-sources` and the `SourceInfo` constructors live in
  `gridwatch-store`, and **`store` must not depend on `sources`**. Either the
  registration site (`sources/src/registry.rs`) overrides the field, or the
  names move to `store`, or `SourceDef` carries them instead of `SourceInfo`.
  This is the one genuinely seam-shaped choice in E1.
- The check would live in `app/src/config.rs` (the loader, which already holds
  `sources: toml::Table`) and be reported by `config_check` in
  `app/src/lib.rs:796` — which, since arc 10a, already builds every component
  and exits non-zero on a failure, so there is a place to hang it.

### Two traps

1. **`refresh_ms` is read outside the `Options` type.** cpu and gpu each have
   their own `cadence_from(options)` (`cpu/mod.rs:49`) that reads it. So the
   accepted set is "what the Options type reads **plus what its helpers read**"
   — which is exactly why `OPTION_NAMES` being hand-written is fine, and why
   deriving the set from a serde struct would be wrong.
2. **Struct field names are not TOML key names.** net's `Options` has
   `refresh`/`link`/`conns_every` for `refresh_ms`/`link_ms`/`conns_ms`. Any
   scheme that derives the accepted set from field names is wrong before it
   starts. (Also: a plain grep for `get("…")` cannot produce the list — audio's
   returns pw-dump JSON fields like `props` and `metadata`, net's returns
   interface names like `eno1`. Established by trying it.)

### Not covered by any of this

Plugin sources. A `[[plugins]]` entry has no `SourceInfo` from the catalogue,
and contract 1 has no message that carries instance options at all (D58
amendment 25 — a plugin tile logs that its options were ignored). Whether
`config check` should say anything about `[sources.<plugin id>]` is a decision.

## E2 — `Record`/`Vector` series are never evicted

### Where things stand after arc 10's review

`Store::sweep` (`store/src/store.rs:119`) **shrinks and never deletes**: a
scalar ring is pruned to its newest point, no series is removed. The first
version emptied rings and dropped entries, and the arc-10 review found it
deleting six catalogued scalars that are published exactly once —
`sensor.max_c`, `sensor.crit_c`, `net.speed_mbps`, `gpu.clock_gfx_max_mhz`,
`gpu.clock_mem_max_mhz`, `gpu.temp_slowdown_c` (D60 review amendment R6). So
the residual leak is now **one map entry plus one point** per label ever seen,
for every series kind — not the 2 400 points it was.

### `KeyMeta` and the label shapes

`KeyMeta` (`store/src/key.rs:163`) has six fields: `name`, `unit`, `kind`,
`source`, `doc`, `decode`. `CATALOGUE` (`:173`) is eight domain slices.
Label shapes as declared per domain:

| domain | labels | dynamic on a real machine? |
|---|---|---|
| `sys`, `media` | none | — |
| `cpu` | `{core}` | no (CPU hotplug only; BACKLOG notes torch never offlines one) |
| `gpu` | `{dev}`, `{dev:fan}`, `{fast\|slow\|procs}` | no |
| `pins` | `{pin}`, `{condition}` | no (one card; multi-card is its own escalation) |
| `audio` | `{ch}` | no |
| `sensors` | `{chip:label}` | **on hotplug** — an NVMe or USB device adds and removes a chip |
| `net` | `{iface}`, `{target}` | **yes** — the case this exists for |

So the flag is `true` for `net.*` certainly, `sensors.*` arguably, and false
for the rest. Whether "arguably" is worth a flag value is a judgment call.

### The decision the backlog entry does not mention: plugins

**Plugin metrics are not in the catalogue at all.** A plugin's sample becomes a
`MetricId` built from wire data at `app/src/plugin/host.rs:610`, so
`key::lookup(name)` returns `None` for it. A `KeyMeta` flag therefore cannot
describe a plugin key — and plugin keys are the *least* bounded of any:

- **names** are interned through `Hosted::intern` (`host.rs:391`) with
  `Box::leak`, capped at `MAX_METRIC_NAMES` (256 per plugin) — bounded;
- **labels** are `Label::Name(Arc<str>)` straight off the wire (`host.rs:597`)
  with no cap at all — a plugin publishing `weather.temp{city}` accrues one
  series per city it has ever seen.

So the sweep needs a rule for "not in the catalogue", and the obvious one
(uncatalogued ⇒ dynamic ⇒ evictable) is a decision with a consequence: it makes
plugin series the *only* ones that can be deleted, which is defensible (a
plugin's tile draws from its own view tree, not from the store) but should be
written down rather than fallen into.

### What the sweep would need

Nothing structural — `sweep` already walks `self.series` and already asks
`crate::rules::label_text(&id.label)`. It would gain a `key::lookup(id.name)`
per series and act on the flag. The cost is one catalogue lookup per series per
sweep (sweeps run every `max_age / 10`, floored at 10 s), against the 2.3 µs a
batch the sweep costs today.

## When to hand back to Opus

Matt asked (2026-09-05) to be told, *inside the Fable session*, when to switch
back. `MODELS.md`'s protocol is `Fable: spec + brief → Opus: implement`, and
`CLAUDE.md`'s escalation list puts "a seam needs changing" and "anything
touches … the store's apply path" on Fable — which is why the two contract
edits themselves stay here rather than going with the breadth.

**Fable's half — the switch-back gate. Say "switch to Opus now" once all five
are true:**

1. **D61 is written**, answering the seven questions below. It is the artifact
   that makes the rest mechanical.
2. **The spec is updated before the code** (D33): `ARCHITECTURE.md` §4.1 for
   `KeyMeta`, §4.3 for `SourceInfo`/`SourceDef` (the signature lines at
   `:101`/`:115`), and §9 if the check changes what a config file means.
3. **A brief exists** in `docs/briefs/` — arc-numbered, since `BACKLOG.md`'s
   header makes pulling these in a DECISIONS entry either way.
4. **Both contract edits compile**: the field on `SourceInfo`/`SourceDef`, the
   flag on `KeyMeta`, and `Store::sweep` reading it. The sweep is the store's
   apply path, so its shape is a Fable call even though it is three lines.
5. **`ROADMAP.md` has the arc's acceptance criteria and gates**, written first
   as D33 requires.

**Opus's half — everything after that is breadth, and is what the switch-back
is for:** the seven source registrations; the loader check and its
warning-vs-error plumbing; `config_check`'s output and exit code (it already
builds every component and exits non-zero since arc 10a, so there is a hook);
generalising `source_and_component_option_names_are_disjoint` from its one
hard-coded pair; tests; regenerating `KEYS.md`/`COMPONENTS.md`; the CHANGELOG
entry; and the arc-end review workflow and gates.

The rule of thumb, if the list above stops matching reality: **hand back when
the remaining work is "do this seven times" rather than "decide what this
should be".**

## What the Fable session still has to decide

1. **Where the option names live** so `store` does not depend on `sources`:
   a field on `SourceInfo` filled at registration, a field on `SourceDef`, or
   moving the names into `store`.
2. Whether the check is a **warning or an error**, and whether `config check`
   exits non-zero on it as it now does for a component that will not build.
3. Whether to fold in the **general disjointness test** (§9's rule is stated
   generally and tested for one pair).
4. Whether `config check` says anything about `[sources.<plugin id>]`.
5. **The `KeyMeta` flag's name and shape** — a `bool`, or something that also
   answers "how long after the last sample".
6. **What the sweep does with an uncatalogued (plugin) key**, and whether
   plugin labels want a cap of their own like plugin names have.
7. Whether `sensors` counts as dynamic.
