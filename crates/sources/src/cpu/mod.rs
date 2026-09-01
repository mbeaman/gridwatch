//! The cpu source (§5 cadence row, §8): procfs meters at 3 s hidden / 1.5 s
//! visible / 500 ms focused, on the shared phase grid. No process scan in arc
//! 1b — `Detail::Table` arrives with the htop `table` tier in arc 2 (§8.1).

pub mod sampler;
pub mod sysfs;

use std::sync::Arc;
use std::time::Duration;

use gridwatch_store::{
    Cadence, Level, Sampler, Source, SourceCtx, SourceInfo, SourceState, SourceStatus, demo,
};

/// How long a paused source parks between checks of the stop flag. It never
/// samples at this cadence — `sleep_until` returns early on any control.
const IDLE_PARK: Duration = Duration::from_secs(5);

pub use sampler::{CpuSampler, Roots, Shares, Ticks, parse_stat, shares};

/// The option names `[sources.cpu]` owns (§9). The htop component's view
/// options must stay disjoint from these — a test in `gridwatch-components`
/// asserts it, because one name meaning two things is a silent misconfiguration.
pub const OPTION_NAMES: &[&str] = &["refresh_ms"];

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
}

impl CpuSource {
    pub fn new(options: &toml::Table) -> CpuSource {
        CpuSource {
            sampler: CpuSampler::new(Roots::default()),
            cadence: cadence_from(options),
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
            && let Ok(samples) = self.sampler.sample(cx.clock.now(), cx.demand.detail())
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
        loop {
            while cx.try_control().is_some() {}
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
            match self.sampler.sample(at, cx.demand.detail()) {
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
