# Adding a component

`ARCHITECTURE.md` §15 is the three-step version. This is the same walk with
every file named, written by following it against the newest real component —
**`sensors`** (arc 5b), which is a source *and* a tile and so touches every
step. Where this disagrees with a file, the file is right and this is a bug:
say so.

If you want a tile that is **not** compiled into gridwatch, you do not want any
of this — you want [`PLUGINS.md`](PLUGINS.md), which is a program in any
language talking JSON over a pipe. This document is for a component that ships
in the binary.

---

## 0. Decide whether you need a source at all

A component reads the store; it never reads a device. So the first question is
whether the numbers you want are already keys.

```console
$ gridwatch keys | grep -i temp
```

If they are, skip to step 2 and write only a tile — `alerts` and `sources` are
both tiles over data somebody else publishes. If they are not, you are adding a
source too, and steps 1a–1d are yours.

---

## 1. The data

### 1a. Keys — `crates/store/src/keys/<domain>.rs`

One file per domain. Every metric is a `Key<T>` constant with a doc comment,
because `gridwatch keys` turns those comments into
[`KEYS.md`](KEYS.md) and CI fails on drift.

```rust
// crates/store/src/keys/sensors.rs
pub const SOURCE: SourceId = SourceId("sensors");

/// `sensor.temp_c{chip:label}` — °C.
pub const TEMP_C: Key<f64> = Key::new("sensor.temp_c");
```

`T` is `f64`, `Vec32`, or a Record type of your own. A **Record** is any
`Serialize + Deserialize + Debug + Send + Sync` struct — that is how the
process tables, the route and the chip inventory travel. A Record needs a
`decode` entry so the journal can round-trip it; the existing domains show the
shape.

Then add the module and its `KeyMeta` slice to the catalogue:

- `crates/store/src/keys/mod.rs` — `pub mod sensors;`
- `crates/store/src/key.rs` — the `CATALOGUE` array of slices.

**Labels.** `Label::Index(u16)` for a core, a pin or a device; `Label::Name`
for an interface or a `chip:label`. Choose once: every alert id, every journal
line and every `{n}` in the docs follows it.

### 1b. The synth — `crates/store/src/demo/`

`--demo`, the snapshot tests and `feed_synth` all run off the same seeded
synth, so a key nothing generates is a key nothing tests. Add yours there and
the tile gets data everywhere at once.

### 1c. The source — `crates/sources/src/<domain>/`

Implement `Source` (or `Sampler` for a plain poller, or `AsyncSource` for one
that needs tokio). The contract is in `ARCHITECTURE.md` §4.3: you get a
`Demand` (how much anyone visible actually wants) and you publish `Batch`es.
Read it before you write a poll loop — the cadence tiers are why the dashboard
costs 1 % of a core rather than 10.

`sensors` is a good model: `hwmon.rs` is a pure walker over a directory tree,
`rapl.rs` is a counter difference, and `mod.rs` is the thread that owns them.
Keeping the parsing pure is what let the tests run against
`fixtures/hwmon/torch/` instead of the machine.

### 1d. Register it — `crates/sources/src/registry.rs`

```rust
#[cfg(feature = "sensors")]
reg.register_source(SourceDef {
    info: gridwatch_store::demo::sensors_info_static(),
    start: crate::sensors::start,
    demo: gridwatch_store::demo::sensors_demo,
});
```

`info` carries the source's id and its visible/focused cadences — the
staleness rule reads them, so a source that lies here gets a `STALE` badge it
does not deserve.

---

## 2. The tile — `crates/components/src/<kind>/`

Copy `clock.rs` if it is small, `sensors/` if it has tiers and a view module.

**The manifest** says what the tile is and what it needs:

```rust
// crates/components/src/sensors/mod.rs, abridged
pub static MANIFEST: Manifest = Manifest {
    kind: "sensors",
    name: "sensors",
    summary: "every hwmon chip's temperatures, fans, volts and power, …",
    footprints: &[Footprint { w: 2, h: 1 }, /* … */ Footprint { w: 6, h: 3 }],
    default_footprint: Footprint { w: 6, h: 1 },
    requires: &[],                                   // nothing is fatal here
    optional: &[Capability::Hwmon, Capability::Rapl], // absent ⇒ say so, still draw
    sources: &[sensors::SOURCE],
    optional_sources: &[gpu::SOURCE],                // renders `—` without it
    keys: &[KeyHint { key: "o", does: "sort hottest / by chip" }],
    ...
};
```

`requires` versus `optional` is the decision worth taking slowly. A **required**
capability that is missing means the tile is never built: the grid draws a
placeholder chip with the reason and the fix from `probe::explain`. An
**optional** one means the tile builds and degrades — which is what `sensors`
does, because a machine with no RAPL still has temperatures worth showing. Ask
"is this tile pointless without it?" and let the answer pick.

`keys` is not decoration: it draws the key bar while your tile has capture, and
it is what [`KEYBINDINGS.md`](KEYBINDINGS.md) generates from. A key you do not
declare is a key nobody can discover.

**Tiers** are cumulative and poorest first, and `tiers()[0].min` must fit
**8×3** — the smallest tile the grid can make. `zoom_only` tiers form a suffix.

```rust
Tier { name: "hottest", min: Size::new(8, 3),  adds: &["the hottest reading"], zoom_only: false },
Tier { name: "table",   min: Size::new(40, 8), adds: &["every chip"],          zoom_only: false },
Tier { name: "full",    min: Size::new(100, 24), adds: &["fans", "volts"],     zoom_only: true },
```

`adds` is prose for `COMPONENTS.md` — what this tier shows that the last one
did not. The **checked** claim is `signature(tier)`, a short list of strings the
testkit asserts really appear at every size that picks that tier:

```rust
fn signature(&self, tier: usize) -> &'static [&'static str] {
    match tier {
        TIER_HOTTEST | TIER_STRIP => &["°"],
        TIER_TABLE => &["chip"],
        _ => &["RAPL"],
    }
}
```

Keep them short and structural — a degree sign, a column header — not a value
that changes with the data. "It did not panic" is not a passing assertion
(D46), and `signature` is how a tier stops being able to silently draw nothing.

**The four methods.**

| method | rule |
|---|---|
| `demand(tier)` | what this tier needs from its sources. Only raise to `Detail::Columns` if you truly need htop's gated files — it is the expensive level. |
| `tick(&mut self, cx)` | derive state when `cx.store.generation(src)` moved. **Sorting, filtering and tree-building live here**, never in `view` (arc 8a's review found a tree rebuilt per row per frame). |
| `view(&self, cx)` | pure over store + `cx.now`. Return a `View` tree; the theme's renderer draws it. |
| `on_key(&mut self, key, cx)` | keys in, `Command`s out. Use `cx.tier` and `cx.zoomed` — **never infer the tier from `inner`'s size** (D58 amendment 7: a 6×3 tile on a big screen clears the `full` tier's minimum, so size alone let zoom-only action keys fire on the grid). |

**What a component may not do:** no I/O, no threads, no device access, no
`Instant::now()` (use `cx.now`), no colour or glyph literals, no cell writes
outside `View::Custom` — and there only through theme roles. Side effects go
out as `Command`s. These are not style rules; they are what makes one render
path work for every theme and what makes the tests deterministic.

---

## 3. Wire it up

Three lines and a feature:

- `crates/components/src/registry.rs` — `reg.register_component((crate::sensors::DEF)());` behind `#[cfg(feature = "sensors")]`
- `crates/cli/Cargo.toml` — the feature, and add it to `default` if it should be on
- `.github/workflows/ci.yml` — the feature matrix checks none / each alone / all, so a new feature that only builds alongside another is caught here

---

## 4. Prove it

```console
$ cargo test -p gridwatch-components
$ scripts/gate.sh --quick
```

The testkit (`gridwatch-ui`, feature `testkit`) gives you the assertions that
matter for free — signatures as they actually are, because they take the tier
list or a constructor rather than a built component:

```rust
assert_min_tier_fits(c.tiers(), Size::new(8, 3));
assert_tiers_well_formed(c.tiers());
// The D46 sweep: every inner size from 0x0 to the richest tier's min plus a
// margin, and the zoomed body, against a populated store *and an empty one*.
assert_renders_everywhere(&|| Box::new(Sensors::default()), &data, &empty, &theme);
```

`assert_renders_everywhere` is the one that earns its keep. Per size it asserts
no panic on either store; that a rect which fits tier 0 draws something
non-blank on **both** (an honest empty tile says `—`, not nothing); that with
data the chosen tier's `signature` strings are present; and that on the empty
store nothing reads as a measured percentage — a tile must not invent `0.0%`
for a number it does not have. "Didn't panic" alone is never a pass
([`TESTING.md`](TESTING.md), layer A).

`snapshot_matrix!` snapshots the view tree *and* the rendered cells at the real
grid sizes.

Then run it for real:

```console
$ cargo run -- --demo            # your tile with synthetic data
$ cargo run -- shot --format cells --size 250x70 | less -R
```

---

## 5. Finish the paperwork

- `docs/PARITY.md` if you are reproducing a tool's behaviour — it is the
  acceptance oracle, and a row that says "out, because…" is a finished row.
- `scripts/shots.sh` regenerates `KEYS.md`, `KEYBINDINGS.md` and
  `COMPONENTS.md`; **commit what it writes**, or CI's drift check fails.
- `docs/PERFORMANCE.md` if your source polls anything — a new poller with no
  measured row is how a budget stops meaning anything.
- `CHANGELOG.md` under `[Unreleased]`.

## The three mistakes worth naming

1. **Deriving state in `view`.** It runs every frame for every visible tile.
   `tick` runs on the same schedule but is allowed to be `&mut`, so that is
   where a sort belongs.
2. **Reading the tier from the rect.** Use `cx.tier`. See the table above.
3. **A tier whose `adds` is aspirational.** The testkit checks those strings at
   every size that picks the tier, which is the point of writing them down.
