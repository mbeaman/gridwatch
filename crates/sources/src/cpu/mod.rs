//! The cpu source (§5 cadence row, §8): procfs meters at 3 s hidden / 1.5 s
//! visible / 500 ms focused, on the shared phase grid, plus the pid-level
//! process scan at `Detail::Table` on its own slower grid — 3 s visible,
//! 1.5 s focused (§8.1, P15) — so a focused tile's 500 ms meters never drag
//! a 12 ms `/proc` walk along with them.

pub mod procs;
pub mod sampler;
pub mod sysfs;

use std::sync::Arc;
use std::time::Duration;

use gridwatch_store::{
    Cadence, Control, Detail, Level, OptionIssue, Sampler, Source, SourceCtx, SourceInfo,
    SourceState, SourceStatus, Ts, demo,
};

use crate::options::Reader;

/// How long a paused source parks between checks of the stop flag. It never
/// samples at this cadence — `sleep_until` returns early on any control.
const IDLE_PARK: Duration = Duration::from_secs(5);

pub use procs::{ProcScanner, Scan};
pub use sampler::{CpuSampler, Roots, Shares, Ticks, parse_stat, shares};

/// The pid-level scan's own cadence (§8.1): 3 s on the grid, 1.5 s focused.
pub fn scan_period(level: Level) -> Duration {
    match level {
        Level::Focused => Duration::from_millis(1500),
        _ => Duration::from_secs(3),
    }
}

/// The option names `[sources.cpu]` owns (§9). The htop component's view
/// options must stay disjoint from these — a test in `gridwatch-components`
/// asserts it, because one name meaning two things is a silent misconfiguration.
/// `k10temp = false` hands `sensor.temp_c{k10temp:*}` to the sensors source
/// (arc 5b, §16); the default follows the build: off with the `sensors`
/// feature, on without it.
pub const OPTION_NAMES: &[&str] = &["refresh_ms", "k10temp"];

pub fn k10temp_default() -> bool {
    !cfg!(feature = "sensors")
}

pub const MIN_REFRESH_MS: i64 = 200;
pub const MAX_REFRESH_MS: i64 = 60_000;
/// The shipped visible cadence, which is also `[sources.cpu] refresh_ms`'s
/// default — §9 writes it out, so the reader can name it in a message.
pub const DEFAULT_REFRESH_MS: i64 = 1500;

/// What `[sources.cpu]` resolves to (D63): the reader's own output, so the
/// declaration of what the key accepts is the code that reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub refresh: Duration,
    /// `false` hands `sensor.temp_c{k10temp:*}` to the sensors source (§16).
    pub k10temp: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            refresh: Duration::from_millis(DEFAULT_REFRESH_MS as u64),
            k10temp: k10temp_default(),
        }
    }
}

impl Options {
    pub fn from_table(t: &toml::Table) -> (Options, Vec<OptionIssue>) {
        let mut r = Reader::new("cpu", t);
        let o = Options::read(&mut r);
        (o, r.finish())
    }

    fn read(r: &mut Reader) -> Options {
        let mut o = Options::default();
        if let Some(ms) = r.int_ms(
            "refresh_ms",
            MIN_REFRESH_MS..=MAX_REFRESH_MS,
            "",
            DEFAULT_REFRESH_MS,
        ) {
            o.refresh = Duration::from_millis(ms as u64);
        }
        // The default follows the build (`k10temp_default`), so the message
        // says what this binary would keep, not a hard-coded word (D63 trap 7).
        if let Some(b) = r.bool("k10temp", o.k10temp) {
            o.k10temp = b;
        }
        o
    }
}

/// The reader `start` runs, without starting anything (§4.3).
pub fn check(t: &toml::Table) -> Vec<OptionIssue> {
    Options::from_table(t).1
}

/// `[sources.cpu] refresh_ms` (§9): the *visible* cadence. Focused stays at
/// htop's fast end, hidden at twice the visible period, both bounded by the
/// reader's clamp so a mistyped config can never spin the poller. At the
/// shipped 1500 ms this is field-identical to `demo::cpu_info().cadence`.
pub fn cadence_from(o: &Options) -> Cadence {
    let visible = o.refresh;
    Cadence {
        hidden: Some(visible * 2),
        visible,
        focused: visible.min(Duration::from_millis(500)),
        always_on: false,
    }
}

/// The live cpu source: a `CpuSampler` plus the demand-driven loop every source
/// runs (level → cadence → phase-aligned deadline → emit).
pub struct CpuSource {
    sampler: CpuSampler,
    cadence: Cadence,
    /// When the next pid-level scan is due; the meters tick that crosses it
    /// samples at `Detail::Table`, every other tick at `Meters`.
    next_scan: Ts,
}

impl CpuSource {
    pub fn new(options: &toml::Table) -> CpuSource {
        CpuSource::from_options(Options::from_table(options).0)
    }

    pub fn from_options(o: Options) -> CpuSource {
        CpuSource {
            sampler: CpuSampler::new(Roots::default()).with_k10temp(o.k10temp),
            cadence: cadence_from(&o),
            next_scan: Ts::ZERO,
        }
    }

    /// The detail this tick samples at: the demanded one when a scan is due,
    /// `Meters` otherwise. The scan grid is phase-aligned like the meters grid
    /// (`next_deadline`): the next multiple of the scan period, so a meters
    /// tick at `k·C + ε` can never miss a scan boundary by its own wake
    /// latency (the review found `at + P` halving the rate at random). A
    /// demand below `Table` resets the schedule so the first tick after a
    /// table tier appears scans immediately, and forgets the per-pid deltas so
    /// that first table shows no percentage rather than one averaged over the
    /// whole absence.
    pub fn detail_for(&mut self, at: Ts, level: Level, wanted: Detail) -> Detail {
        if wanted < Detail::Table {
            if self.next_scan != Ts::ZERO {
                self.next_scan = Ts::ZERO;
                self.sampler.forget_scan_deltas();
            }
            return wanted;
        }
        if at >= self.next_scan {
            let p = scan_period(level).as_nanos().max(1) as u64;
            self.next_scan = Ts(((at.0 / p) + 1) * p);
            wanted
        } else {
            Detail::Meters
        }
    }
}

impl Source for CpuSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: self.cadence,
            ..demo::cpu_info()
        }
    }

    fn run(mut self: Box<Self>, cx: SourceCtx) {
        let mut state = SourceState::Starting;
        cx.status(SourceStatus {
            state,
            reason: None,
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: cx.restarts,
        });
        // Prime the pump before the first sleep: P18 gates "every source live"
        // at 2 s, and waiting for the first cadence boundary (3 s while the
        // demand is still Hidden) missed it. It also brings the first *delta*
        // forward by a whole period, so percentages appear on the second scan
        // rather than the third.
        // A paused source emits nothing, restart or not (§4.3).
        if cx.demand.level() != Level::Paused
            && let Ok(samples) = {
                let at = cx.clock.now();
                let detail = self.detail_for(at, cx.demand.level(), cx.demand.detail());
                self.sampler.sample(at, detail)
            }
            && !samples.is_empty()
        {
            let at = cx.clock.now();
            cx.emit(at, samples);
            state = SourceState::Ok;
            cx.status(SourceStatus {
                state,
                reason: None,
                hint: None,
                since: at,
                last_sample: Some(at),
                dropped: 0,
                restarts: cx.restarts,
            });
        }
        // htop's `H` (arc 10b, D60). The flag cannot ride `Detail`: the I/O
        // screen raises `Detail::Columns` too and wants no `task/` walk. It is
        // process-wide, so it survives a supervisor restart the way `Demand`
        // does — the arc-10 review found a per-sampler flag came back `false`
        // after a panic while the tile still said `H` was on.
        let threads = self.sampler.threads_flag();
        loop {
            while let Some(c) = cx.try_control() {
                if let Control::SetOption(k, v) = c
                    && k == "threads"
                    && let Some(on) = v.as_bool()
                {
                    threads.store(on, std::sync::atomic::Ordering::Relaxed);
                }
            }
            if cx.stopped() {
                return;
            }
            let level = cx.demand.level();
            // `None` means *do not poll* (§4.3): park on the control channel
            // instead of waking every second to decide not to sample.
            let Some(period) = self.cadence.for_level(level) else {
                if !cx.sleep_until(cx.next_deadline(IDLE_PARK)) {
                    return;
                }
                continue;
            };
            if !cx.sleep_until(cx.next_deadline(period)) {
                return;
            }
            let at = cx.clock.now();
            // Re-read the level with the detail: the demand may have moved
            // while we slept, and the two fields are separate atomics.
            let detail = self.detail_for(at, cx.demand.level(), cx.demand.detail());
            match self.sampler.sample(at, detail) {
                Ok(samples) => {
                    let empty = samples.is_empty();
                    if !empty {
                        cx.emit(at, samples);
                    }
                    if state != SourceState::Ok && !empty {
                        state = SourceState::Ok;
                        cx.status(SourceStatus {
                            state,
                            reason: None,
                            hint: None,
                            since: at,
                            last_sample: Some(at),
                            dropped: 0,
                            restarts: cx.restarts,
                        });
                    }
                }
                Err(e) => {
                    if state != SourceState::Unavailable {
                        state = SourceState::Unavailable;
                        cx.status(SourceStatus {
                            state,
                            reason: Some(Arc::from(e.reason.as_str())),
                            hint: e.hint.as_deref().map(Arc::from),
                            since: at,
                            last_sample: None,
                            dropped: 0,
                            restarts: cx.restarts,
                        });
                    }
                }
            }
        }
    }
}

/// `SourceDef.start` for the registry. Every issue the reader found is
/// logged once here, at the moment the source starts on its defaults — the
/// log keeps the lines the scattered `warn!`s used to write, and the screen
/// gains them through `check` (D63).
pub fn start(options: &toml::Table) -> Box<dyn Source> {
    let (o, issues) = Options::from_table(options);
    crate::options::log("cpu", &issues);
    Box::new(CpuSource::from_options(o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::only_issue;
    use gridwatch_store::IssueKind;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    /// D63 trap 3: the reader must ask for exactly the accepted set, in
    /// order — a key `OPTION_NAMES` carries and nothing reads is silence of
    /// the kind this arc exists to end, and a key the reader reports that
    /// the set does not carry would fail arc 11's name pass on the same file.
    #[test]
    fn the_reader_asks_for_exactly_the_accepted_set() {
        let t = toml::Table::new();
        let mut r = Reader::new("cpu", &t);
        let o = Options::read(&mut r);
        assert_eq!(r.asked(), OPTION_NAMES);
        assert_eq!(o, Options::default());
        assert!(r.finish().is_empty(), "an empty table is not a problem");
    }

    /// The acceptance case (ROADMAP arc 13): the string is discarded, and the
    /// message says what stands instead of leaving the reader to guess.
    #[test]
    fn a_wrongly_typed_refresh_keeps_the_default_and_says_so() {
        let (kind, text) = only_issue(check(&table("refresh_ms = \"1500\"")));
        assert_eq!(kind, IssueKind::Rejected);
        assert_eq!(
            text,
            "`refresh_ms` expects an integer (milliseconds), found a string (\"1500\") \
             — the default 1500 stands"
        );
        assert_eq!(
            Options::from_table(&table("refresh_ms = \"1500\""))
                .0
                .refresh,
            Options::default().refresh
        );
    }

    #[test]
    fn a_too_fast_refresh_is_clamped_and_used() {
        let (kind, text) = only_issue(check(&table("refresh_ms = 50")));
        assert_eq!(kind, IssueKind::Adjusted);
        assert_eq!(text, "`refresh_ms` = 50 clamped to 200 (accepts 200-60000)");
        let (o, _) = Options::from_table(&table("refresh_ms = 50"));
        assert_eq!(o.refresh, Duration::from_millis(200));
    }

    /// `k10temp`'s default follows the build (§16), so the sentence is
    /// composed from it and never hard-coded (D63 trap 7).
    #[test]
    fn k10temp_names_this_builds_default() {
        let (kind, text) = only_issue(check(&table("k10temp = \"yes\"")));
        assert_eq!(kind, IssueKind::Rejected);
        assert_eq!(
            text,
            format!(
                "`k10temp` expects a boolean, found a string (\"yes\") — the default {} stands",
                k10temp_default()
            )
        );
    }

    /// The shipped cadence must not move: `demo::cpu_info()` is what a demo
    /// and a journal source run at, and P15's rows are taken at it.
    #[test]
    fn the_shipped_refresh_reproduces_the_registered_cadence() {
        let (a, b) = (cadence_from(&Options::default()), demo::cpu_info().cadence);
        assert_eq!(
            (a.hidden, a.visible, a.focused, a.always_on),
            (b.hidden, b.visible, b.focused, b.always_on)
        );
    }
}
