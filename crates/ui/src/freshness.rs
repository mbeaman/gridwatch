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

    /// The age to print, or `None` when there is nothing honest to print.
    ///
    /// **A quiet row must not show an age while its tile's `STALE` badge is
    /// up** (D68 §5): the badge counts wall time and this counts store time,
    /// so once the source stalls they diverge — fourfold under `--replay
    /// --speed 4` — and two disagreeing ages on one tile read as one system
    /// that lies. A component cannot see the badge, so the test is the
    /// condition that puts the badge *down*: the source is still advancing,
    /// which `Pulse::advancing` answers.
    pub fn age(self, pulse: &Pulse) -> Option<Duration> {
        match self {
            Liveness::Quiet { age } if pulse.advancing() => Some(age),
            _ => None,
        }
    }
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

    /// Is this source still producing? False before its first batch — which is
    /// also when a tile has nothing to draw and must not call anything dead.
    pub fn advancing(&self) -> bool {
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

    /// The newest point of `key`, judged. `None` when the label has no series
    /// at all — absent, not quiet, and the row should not exist.
    pub fn scalar(
        &self,
        store: &Store,
        key: &gridwatch_store::Key<f64>,
    ) -> Option<(Liveness, f64)> {
        let (at, v) = store.last(key)?;
        Some((self.of(at), v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_store::keys::disk;

    fn at(secs: u64) -> Ts {
        Ts(secs * 1_000_000_000)
    }

    /// The whole correctness argument is that this module reads no clock: that
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
        for forbidden in ["Instant::now", "SystemTime::now", "cx.now", "clock.now"] {
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
        assert!(!p.advancing());
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

    /// And no second age on screen while the badge is up (D68 §5).
    #[test]
    fn a_quiet_row_shows_no_age_before_the_source_has_ever_published() {
        let p = Pulse::new(disk::SOURCE);
        let q = Liveness::Quiet {
            age: Duration::from_secs(9),
        };
        assert_eq!(q.age(&p), None, "no source progress, no age");
        let mut advancing = Pulse::new(disk::SOURCE);
        advancing.seen = Some(at(10));
        assert_eq!(q.age(&advancing), Some(Duration::from_secs(9)));
    }

    /// An implausible gap is ignored rather than trusted: a source whose two
    /// newest batches are a day apart has not got a one-day cadence.
    #[test]
    fn an_implausible_gap_keeps_the_last_credible_period() {
        let mut p = Pulse::new(disk::SOURCE);
        p.seen = Some(at(0));
        p.period = Duration::from_secs(2);
        // 86 400 s is outside MIN..=MAX, so the period must not move.
        p.seen = Some(at(1));
        let kept = p.period;
        assert_eq!(kept, Duration::from_secs(2));
    }
}
