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
    Cadence, Control, Detail, Level, Sampler, Source, SourceCtx, SourceInfo, SourceState,
    SourceStatus, Ts, demo,
};

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

/// `[sources.cpu] refresh_ms` (§9): the *visible* cadence. Focused stays at
/// htop's fast end, hidden at twice the visible period, both clamped so a
/// mistyped config can never spin the poller.
fn cadence_from(options: &toml::Table) -> Cadence {
    let base = demo::cpu_info().cadence;
    let Some(ms) = options
        .get("refresh_ms")
        .and_then(|v| v.as_integer())
        .filter(|ms| *ms > 0)
    else {
        return base;
    };
    let visible = Duration::from_millis((ms as u64).clamp(200, 60_000));
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
        let k10temp = options
            .get("k10temp")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(k10temp_default);
        CpuSource {
            sampler: CpuSampler::new(Roots::default()).with_k10temp(k10temp),
            cadence: cadence_from(options),
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
        // screen raises `Detail::Columns` too and wants no `task/` walk.
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

/// `SourceDef.start` for the registry.
pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(CpuSource::new(options))
}
