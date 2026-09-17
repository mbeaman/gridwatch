# Arc 17 brief — "a thing that goes quiet says so" (D68)

> Written 2026-09-16 with D68. **Read D68 first** — it decides everything here, and it was
> chosen *whole* from three Fable designs rather than blended, so resist improving it with
> pieces of the other two. **Arc 17 changes no contract.** Nothing in `store`, `sources`,
> `app`, `Msg`, `RenderCx`, `TickCx`, `View` or the journal moves. If anything here seems to
> need that, **stop and escalate** — the store-side version is D68's rejected alternative and
> the reasons are written down.

## What already exists (do not rebuild)

| thing | where | state |
|---|---|---|
| `Store::last(&Key<f64>) -> Option<(Ts, f64)>` | `store/src/store.rs:373` | **the timestamp has been there since arc 1** and nothing reads it |
| `Store::last_sample(SourceId) -> Option<Ts>` | `store/src/store.rs:453` | the source's newest batch clock — the reference point |
| `Ts::since(earlier) -> Duration` | `store/src/ts.rs:29` | saturating; zero if `earlier` is later |
| the exact `Pulse` loop | `components/src/disk/mod.rs:554-569` | `tick` already does newest-sample + observed-gap + clamp. **Move it, do not re-derive it** |
| `MIN_CADENCE` / `MAX_CADENCE` / `DEFAULT_CADENCE` | `components/src/disk/mod.rs:128-131` | the clamp range; reuse the values |
| the bullet vocabulary | `components/src/net/view.rs:39-46` | `● Ok / ◍ Warn / ○ TextMuted / · TextGhost`, `·` already means "nothing known" |
| `overlay::stale_age_text` | `ui/src/overlay.rs` | the `12s` / `14m` formatter. Reuse it — **but see seam 4** |
| `overlay::dim` | `ui/src/overlay.rs:173-181` | inserts `Modifier::DIM` *because* mono's muted and plain text are the same colour |
| `removable_present(at)` | `store/src/demo/disk.rs` | the unplug hook. Its window is too short — seam 5 |

## Seam 1 — `crates/ui/src/freshness.rs` (new)

```rust
pub const STALE_PERIODS: u32 = 3;                    // §11's constant
pub struct Pulse { source: SourceId, seen: Option<Ts>, period: Duration }
pub enum Liveness { Live, Quiet { age: Duration } }

impl Pulse {
    pub fn new(source: SourceId) -> Pulse;                       // period = DEFAULT
    pub fn observe(&mut self, store: &Store) -> Option<Ts>;      // in `tick`; None = nothing new
    pub fn period(&self) -> Duration;
    pub fn hold(&self) -> Duration;                              // period * STALE_PERIODS
    pub fn of(&self, at: Ts) -> Liveness;                        // against `self.seen`
}
```

`of` is: `match self.seen { Some(s) if s.since(at) > self.hold() => Quiet { age: s.since(at) }, _ => Live }`.
`observe` is `disk/mod.rs:554-569` lifted verbatim, returning the new `Ts`. **No `now` anywhere
in this file** — that is what makes replay, pause and focus-loss correct for free, and a test
should assert the module contains no clock call.

## Seam 2 — the three tiles

Each keeps one `Pulse` for the source it reads, replaces its private staleness with
`Pulse::of`, and gains one field per row (`live: Liveness`).

- **`disk`** — `Drive.live`. `Model::refresh` takes the `Pulse` and judges each device on
  its **anchor key** `disk.read_bps{name}` (published every tick for every present device).
  **Keep `cadence`, `MIN/MAX_CADENCE` and `await_hold`** — D68 §4; the await hold is a
  different question and deleting it makes those columns strobe.
- **`net`** — `Iface.live`, anchor `net.rx_bps{iface}`. Note a *down* interface still
  publishes (rx 0, link down) and is `○ TextMuted` already — quiet is a third thing: absent
  from `/proc/net/dev` entirely.
- **`sensors`** — delete `STALE_AFTER` and the `now.since(at) > 5s` drop. Anchor
  `sensor.temp_c{chip:label}`. **The `Pulse` is not necessarily on the sensors source**: with
  `[sources.cpu] k10temp = true` — the default — the *cpu* source publishes
  `sensor.temp_c{k10temp:*}` (D68 §3). Judge each reading against the source that carried it,
  or every k10temp row flickers. This is the one place the design is subtle; get it right or
  say in the report that you could not.

## Seam 3 — what a quiet row draws

Numbers → `—`. Bullet → `·` in `Role::TextGhost`. Identity cell keeps its name.
**Sink below every live row under every sort**, keeping the sort's own order among the quiet.
Where an elastic column has room, the age.

At the 8×3 `rates` chip and `sparks`: **exclude quiet devices from the totals** — the frozen
`98M` is currently inside the sum — and name that blank in `ARCHITECTURE.md` §8 per D65 §9.
The chart needs nothing: its line already stops.

## Seam 4 — one age on screen, never two

**A quiet row shows no age while its tile's `STALE` badge is up.** The badge counts wall
time; a quiet row counts store time; when a source stalls after a device leaves they diverge,
fourfold under `--replay --speed 4`. The component cannot see the badge, so the honest form
is: the age is drawn only when the source is currently advancing — `Pulse::observe` returned
`Some` within the last period — which is exactly the condition under which the badge is down.
State how you did it; if you cannot, draw no age at all and say so.

## Seam 5 — a synth that actually unplugs

`demo::DiskSynth`'s removable leaves for **15 s** against a retention floor of **60 s**, so
it cannot reach the path arc 16 added it for — arc 16's own review measured 96 consecutive
frames with the device present. Lengthen the absence past the floor. The disk synth's cycle
then disagrees with `net`'s 60 s; that is acceptable and must be written in the code, because
a replay that repeats as a whole was the reason they matched.

Without this the arc ships **unpinned** and repeats the lesson of the arc it cites.

## Numbers

- `STALE_PERIODS = 3`; disk cadence 1 s visible, so a device reads quiet ≈ 3 s after it goes.
- `MIN_CADENCE` 250 ms, `MAX_CADENCE` 10 s, `DEFAULT_CADENCE` 1 s.
- Retention floor: `[store] history` clamps to 1 m, so a quiet row survives ≥ 1 minute before
  the sweep removes the label and the row vanishes with no transition. Say this in §11.

## Traps — do not rediscover

1. **Do not delete disk's cadence observer.** It looks like a duplicate of `Pulse` and is not
   (D68 §4). Two mechanisms, deliberately.
2. **Do not reach for `cx.now`.** The whole correctness argument is that this reads no clock.
   A test should fail if `freshness.rs` grows one.
3. **The sensors `Pulse` may be on the cpu source.** Seam 2.
4. **Sensors snapshots will churn**: rows the 5 s literal dropped come back. That is the fix,
   not a regression — but check each diff rather than accepting in bulk (`REVIEW.md`'s gate
   checklist says one by one, and arc 16 did not).
5. **`assert_renders_everywhere` runs on an empty store**, where `last_sample` is `None`.
   `Pulse::of` must answer `Live` then, not quiet — a tile with no data yet must not draw
   every row as dead.
6. **The unenforced premise.** "Every source names every live label in every batch" holds for
   every shipped source and nothing makes it hold. Add a registry test that asserts it, so a
   future source that violates it fails here rather than flickering in front of someone.

## Escalate rather than improvise

Any of these means stop: needing a field on `RenderCx`/`TickCx`; needing the store to
remember anything; needing a new `View` variant or `Role`; needing a source to change.

## Gates

`scripts/gate.sh` green. `replaying_a_fixture_twice_is_byte_identical` unchanged. P18 and P19
asserted unchanged and **not** re-derived — nothing is added to `Store::apply` or the frame
path, which is the reason this design was chosen.

## Done when

In a pty: unplug the demo's removable, and within three seconds its row reads dashes, wears
`·`, and sits below every live row — under `sort = traffic` *and* `sort = name`. Then stall
the source: the badge appears and the row does **not** grow a second, differently-clocked age.
