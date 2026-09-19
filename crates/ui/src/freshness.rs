//! Per-label liveness: is this *thing* still being reported, inside a source
//! that is itself perfectly healthy? (D68.)
//!
//! §11's `STALE` badge answers a different question — *is this source late* —
//! and is keyed to `SourceStatus.last_sample`. So when one drive is unplugged
//! while three others keep reporting, the badge cannot fire and the vanished
//! drive's row keeps its last numbers for as long as retention holds them.
//! Measured in a real terminal: `98M · 93% busy`, the bullet a green `Role::Ok`
//! byte-identical live and stale, holding second place in a traffic sort.
//!
//! **The rule is measured against the source's own progress, never a clock:**
//!
//! ```text
//! behind = last_sample(source) − label.last_seen
//! quiet  ⇔ behind > STALE_PERIODS × the observed period
//! ```
//!
//! That is what makes the awkward cases correct without a single special case.
//! When a source stalls, is paused, loses focus, is parked, or a replay ends,
//! its `last_sample` stops advancing — so `behind` freezes, nothing goes quiet,
//! and §11's badge says the true thing instead. `app.rs`'s `stale_age` spells
//! those four exemptions out one by one precisely because it reads the wall
//! clock. **Nothing here reads a clock**, which is also why replay and live
//! agree and why `replaying_a_fixture_twice_is_byte_identical` is untouched.
//!
//! The unit is the **label**, not the series. `sensor.max_c`, `net.speed_mbps`,
//! the static gpu clocks and `disk.info` are published once and are perfectly
//! current for as long as their device is present; ageing series individually
//! would call every one of them stale (D61's R6 again). Judge a label on an
//! *anchor* key its source publishes for every live label on every batch —
//! `disk.read_bps`, `net.rx_bps`, `sensor.temp_c`.
//!
//! **There is no age here, and that is deliberate.** A quiet row says `gone`
//! and never *how long*. The age the first pass drew was the store-time gap
//! at the instant the source last spoke; once the source stalls it froze
//! beside the `STALE` badge, which counts wall time, so one tile showed two
//! ages on two clocks (arc 17 review, captured in a pty: `gone 14s` beside
//! `STALE 30s`). Drawing the age only while the badge is down needs to know
//! whether the source is *currently* publishing, which is a clock — the one
//! thing this module's correctness argument says it does not have. `BACKLOG.md`
//! keeps the question of where that clock may live for a design session.
//!
//! What this is **not**: the per-series holds inside a live label. A drive
//! doing one I/O a second publishes `read_bps` every tick and `read_await_ms`
//! on almost none (D64 trap 7); an unreachable probe publishes `loss_pct`
//! every tick and `rtt_ms` never. Those holds stay where they are. Two
//! mechanisms, deliberately (D68 §4).

use std::time::Duration;

use gridwatch_store::{SourceId, Store, Ts};

/// How many of a source's own periods a label may be missing before it is
/// quiet. §11's existing constant, which two tiles had already reached
/// independently before this module existed.
pub const STALE_PERIODS: u32 = 3;

/// The period assumed until two batches have been seen.
pub const DEFAULT_PERIOD: Duration = Duration::from_secs(1);
/// The range an observed period is trusted in — the gpu's fastest default at
/// one end, a slow source's at the other. Outside it, keep the last good one.
const MIN_PERIOD: Duration = Duration::from_millis(250);
const MAX_PERIOD: Duration = Duration::from_secs(10);

/// One source's heartbeat as a component sees it: its newest batch and the
/// observed gap between the two newest. Held by the component, fed in `tick`.
#[derive(Clone, Copy, Debug)]
pub struct Pulse {
    source: SourceId,
    seen: Option<Ts>,
    period: Duration,
}

/// What a component should say about one label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Liveness {
    /// Reported within the last `STALE_PERIODS` of its source's progress.
    #[default]
    Live,
    /// The source has moved on `age` without mentioning it.
    Quiet { age: Duration },
}

impl Liveness {
    pub fn is_quiet(self) -> bool {
        matches!(self, Liveness::Quiet { .. })
    }
}

/// The kindest verdict among the sources that could have published a label,
/// used where the store does not record which one did.
///
/// A label is quiet only when **every** candidate source that has published
/// has moved on without it; one that still names it keeps it live. Adding a
/// candidate can therefore only make a verdict *less* likely to be quiet, so a
/// mistake in the candidate list holds a gone device a little too long rather
/// than declaring a live one dead.
///
/// **Who is a candidate is the caller's decision, and it must be narrow.**
/// D68 §3 said the cpu source publishes `sensor.temp_c{k10temp:*}` "by
/// default"; it does not. `k10temp = true` is the default only when the
/// `sensors` feature is compiled *out*, and `sensors` is a default feature, so
/// in the shipped build the sensors source carries every reading. A wider
/// candidate list is not free: a source that never carried the label still
/// votes, and a slow one (`refresh_ms` up to a minute) votes "live" for a gone
/// chip for as long as its next batch is far off. Name a second source only for the labels it
/// can actually publish.
///
/// A source that has **never published** is not a candidate: it cannot have
/// "moved on without" anything, and counting it would mean nothing is ever
/// quiet in a build where one of the sources is absent.
pub fn judge(pulses: &[&Pulse], at: Ts) -> Liveness {
    let mut quietest: Option<Duration> = None;
    for p in pulses.iter().filter(|p| p.has_published()) {
        match p.of(at) {
            Liveness::Live => return Liveness::Live,
            Liveness::Quiet { age } => {
                quietest = Some(quietest.map_or(age, |w: Duration| w.min(age)));
            }
        }
    }
    quietest.map_or(Liveness::Live, |age| Liveness::Quiet { age })
}

impl Pulse {
    pub fn new(source: SourceId) -> Pulse {
        Pulse {
            source,
            seen: None,
            period: DEFAULT_PERIOD,
        }
    }

    /// Call once per `tick`. `Some(at)` when the source has published
    /// something this component has not seen — the existing "rebuild now"
    /// signal — and the period is updated from the gap when it is credible.
    pub fn observe(&mut self, store: &Store) -> Option<Ts> {
        let at = store.last_sample(self.source)?;
        if self.seen == Some(at) {
            return None;
        }
        if let Some(prev) = self.seen {
            let gap = at.since(prev);
            if (MIN_PERIOD..=MAX_PERIOD).contains(&gap) {
                self.period = gap;
            }
        }
        self.seen = Some(at);
        Some(at)
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    /// How long a label may go unmentioned before it is quiet.
    pub fn hold(&self) -> Duration {
        self.period * STALE_PERIODS
    }

    /// Has this source published at all? False before its first batch — which
    /// is also when a tile has nothing to draw and must not call anything dead.
    ///
    /// This is **not** "is the source still producing": once true it stays
    /// true, and nothing here can say a source has *stopped* without a clock.
    /// It was named `advancing` and used to decide whether a quiet row could
    /// show an age beside the tile's `STALE` badge, which it cannot decide.
    pub fn has_published(&self) -> bool {
        self.seen.is_some()
    }

    /// Judge a reading stamped `at`.
    ///
    /// Answers `Live` before the source's first batch: an empty store is not
    /// evidence that everything in it is dead, and `assert_renders_everywhere`
    /// sweeps every component against exactly that store.
    pub fn of(&self, at: Ts) -> Liveness {
        match self.seen {
            Some(seen) if seen.since(at) > self.hold() => Liveness::Quiet {
                age: seen.since(at),
            },
            _ => Liveness::Live,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_store::keys::disk;
    use gridwatch_store::{Batch, Msg};

    fn at(secs: u64) -> Ts {
        Ts(secs * 1_000_000_000)
    }

    /// The whole correctness argument is that this module reads no clock. **This
    /// is a tripwire on spellings, not a proof** — a `now: Ts` parameter or an
    /// aliased import would pass it — which is why `judge` and `observe` are
    /// also tested with `Ts` values no clock could have produced. That
    /// is what makes pause, focus-loss, a parked source and a finished replay
    /// correct without a single special case, and what keeps replay and live
    /// in agreement. `app.rs`'s wall-clock `stale_age` needs four exemptions
    /// written out; this needs none. Assert it on the source text so a future
    /// edit cannot quietly add one (D68 §1).
    #[test]
    fn this_module_reads_no_clock() {
        // Only the module above `#[cfg(test)]`: this test's own needle list
        // lives in the same file, so scanning the whole of it always matches
        // itself. (Caught by running it — the first version failed on its own
        // string literals.)
        let whole = include_str!("freshness.rs");
        let src = whole.split("#[cfg(test)]").next().expect("a module body");
        for forbidden in [
            "Instant::now",
            "SystemTime::now",
            "UNIX_EPOCH",
            ".elapsed()",
            "Clock::",
            "Ts::now",
            "cx.now",
            "clock.now",
            "now()",
        ] {
            assert!(
                !src.contains(forbidden),
                "`{forbidden}` in freshness.rs — the design rests on this module \
                 measuring store time against a source's own progress, never a clock"
            );
        }
    }

    /// Before a source's first batch a tile has nothing, and nothing is not
    /// evidence that everything is dead. `assert_renders_everywhere` sweeps
    /// every component against exactly this store (brief trap 5).
    #[test]
    fn an_empty_store_makes_nothing_quiet() {
        let p = Pulse::new(disk::SOURCE);
        assert!(!p.has_published());
        assert_eq!(p.of(at(0)), Liveness::Live);
        assert_eq!(p.of(at(9_999)), Liveness::Live);
    }

    /// The rule itself, against a hand-built heartbeat: three periods is the
    /// boundary and it is exclusive.
    #[test]
    fn quiet_after_three_of_the_sources_own_periods() {
        let mut p = Pulse::new(disk::SOURCE);
        p.seen = Some(at(10));
        p.period = Duration::from_secs(1);
        assert_eq!(p.hold(), Duration::from_secs(3));
        assert_eq!(p.of(at(10)), Liveness::Live, "this batch");
        assert_eq!(
            p.of(at(7)),
            Liveness::Live,
            "exactly three periods is not yet"
        );
        assert_eq!(
            p.of(at(6)),
            Liveness::Quiet {
                age: Duration::from_secs(4)
            },
            "four periods behind"
        );
    }

    /// The threshold follows the source rather than a constant: the same
    /// four-second gap is quiet at a 1 s cadence and live at a 5 s one. This
    /// is the reason the design needs no configuration.
    #[test]
    fn the_threshold_follows_the_source_not_a_number() {
        let mut fast = Pulse::new(disk::SOURCE);
        fast.seen = Some(at(100));
        fast.period = Duration::from_secs(1);
        let mut slow = Pulse::new(disk::SOURCE);
        slow.seen = Some(at(100));
        slow.period = Duration::from_secs(5);
        assert!(fast.of(at(96)).is_quiet());
        assert!(!slow.of(at(96)).is_quiet());
    }

    /// A stalled source must mark nothing quiet — its `last_sample` stops, so
    /// every label stays exactly as behind as it was. This is §11's case and
    /// the badge's, and the reason no exemption list is needed here.
    #[test]
    fn a_stalled_source_marks_nothing_quiet() {
        let mut p = Pulse::new(disk::SOURCE);
        p.seen = Some(at(10));
        p.period = Duration::from_secs(1);
        let before = p.of(at(10));
        // Time passes in the world; the source publishes nothing, so `seen`
        // does not move and neither does the verdict.
        assert_eq!(p.of(at(10)), before);
        assert_eq!(before, Liveness::Live);
    }

    /// The period is **observed**, and only credible gaps are believed. This
    /// used to assign `seen` by hand and assert a field it had just set, so
    /// deleting the clamp — or the period update altogether — left it green
    /// (arc 17 review, lens E). It runs `observe` against a real store now.
    #[test]
    fn the_observed_period_follows_credible_gaps_and_ignores_implausible_ones() {
        let mut store = Store::default();
        let mut p = Pulse::new(disk::SOURCE);
        let mut feed = |p: &mut Pulse, ms: u64| {
            store.apply(&Msg::Batch(Batch {
                source: disk::SOURCE,
                at: Ts(ms * 1_000_000),
                samples: vec![],
            }));
            p.observe(&store)
        };
        assert_eq!(p.observe(&Store::default()), None, "nothing published yet");
        assert!(feed(&mut p, 1_000).is_some());
        assert_eq!(p.period(), DEFAULT_PERIOD, "one batch is not a gap");
        feed(&mut p, 3_000);
        assert_eq!(p.period(), Duration::from_secs(2), "a 2 s gap is credible");
        assert_eq!(p.hold(), Duration::from_secs(6));
        feed(&mut p, 3_100);
        assert_eq!(
            p.period(),
            Duration::from_secs(2),
            "a 100 ms gap is below the floor: two batches in one burst are not a cadence"
        );
        feed(&mut p, 3_100 + 86_400_000);
        assert_eq!(
            p.period(),
            Duration::from_secs(2),
            "a day-long gap is not a one-day cadence"
        );
        feed(&mut p, 3_100 + 86_400_000 + 1_500);
        assert_eq!(
            p.period(),
            Duration::from_millis(1_500),
            "and it follows a faster cadence"
        );
        assert_eq!(p.observe(&store), None, "nothing new since the last batch");
    }

    fn pulse(seen: u64, period_s: u64) -> Pulse {
        let mut p = Pulse::new(disk::SOURCE);
        p.seen = Some(at(seen));
        p.period = Duration::from_secs(period_s);
        p
    }

    /// `judge` is the kindest verdict, and this is the arc's one subtle
    /// function. It had no test at all in the first pass — replacing
    /// `return Live` with `continue` left every test green.
    #[test]
    fn judge_is_the_kindest_verdict_among_the_sources_that_have_spoken() {
        // Both are 9 s / 19 s behind their own progress, but the slow source's
        // hold is 30 s.
        let fast = pulse(10, 1); // hold 3 s: `at(1)` is 9 s behind → quiet
        let slow = pulse(10, 10); // hold 30 s: `at(1)` is 9 s behind → live
        assert!(fast.of(at(1)).is_quiet() && !slow.of(at(1)).is_quiet());
        assert_eq!(
            judge(&[&fast, &slow], at(1)),
            Liveness::Live,
            "one live voice is enough"
        );
        assert!(judge(&[&fast], at(1)).is_quiet());

        // Everyone has moved on: quiet, and the *smallest* age is reported
        // (the least alarming true statement).
        let other = pulse(20, 1);
        assert_eq!(
            judge(&[&fast, &other], at(1)),
            Liveness::Quiet {
                age: Duration::from_secs(9)
            }
        );
    }

    #[test]
    fn judge_ignores_a_source_that_has_never_spoken_and_an_empty_list() {
        let fast = pulse(10, 1);
        let silent = Pulse::new(disk::SOURCE);
        assert!(
            judge(&[&fast, &silent], at(1)).is_quiet(),
            "a source that never published cannot have moved on without anything, \
             and must not veto"
        );
        assert_eq!(judge(&[], at(1)), Liveness::Live);
        assert_eq!(
            judge(&[&silent], at(1)),
            Liveness::Live,
            "no votes is not a verdict"
        );
    }
}
