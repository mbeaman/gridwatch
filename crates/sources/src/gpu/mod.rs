//! The gpu source (§5 cadence row, §8, brief 2b task 1): NVML on its own
//! thread — fast tier 1 s hidden / 500 ms visible / 250 ms focused, the slow
//! tier on a 1 s grid, fans every 5 s, `samples(Power)` while a gpu tile is
//! visible, process rows only at `Detail::Table` — with per-field pruning and
//! the degraded states of §11: `LibloadingError` → the nvidia-smi CSV tier,
//! `LibRmVersionMismatch` → `Unavailable` with no retry, `GpuLost` → re-init
//! with backoff. The tier logic is in `poller` over the `probe` seam so it is
//! tested without a GPU; `nvml` and `smi` are the two backends.

pub mod nvml;
pub mod poller;
pub mod probe;
pub mod procs;
pub mod smi;
pub mod specs;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gridwatch_store::{
    Cadence, Level, Source, SourceCtx, SourceInfo, SourceState, SourceStatus, Ts, demo,
};

pub use poller::{FAN_PERIOD, PROCS_PERIOD, Plan, Poller, SLOW_PERIOD};
pub use probe::{Fail, Probe};

const IDLE_PARK: Duration = Duration::from_secs(5);
const DEVICE: u16 = 0;

/// The option names `[sources.gpu]` owns (§9): `refresh_ms` is the visible
/// fast-tier cadence; `device` picks the NVML index (default 0).
pub const OPTION_NAMES: &[&str] = &["refresh_ms", "device"];

fn cadence_from(options: &toml::Table) -> Cadence {
    let base = demo::gpu_info().cadence;
    let Some(ms) = options
        .get("refresh_ms")
        .and_then(|v| v.as_integer())
        .filter(|ms| *ms > 0)
    else {
        return base;
    };
    let visible = Duration::from_millis((ms as u64).clamp(100, 60_000));
    Cadence {
        hidden: Some(visible.max(Duration::from_secs(1))),
        visible,
        focused: (visible / 2).max(Duration::from_millis(100)),
        always_on: false,
    }
}

/// CPU wall clock in microseconds — `process_utilization_stats`' currency.
fn wall_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

pub struct GpuSource {
    cadence: Cadence,
    index: u32,
}

impl GpuSource {
    pub fn new(options: &toml::Table) -> GpuSource {
        GpuSource {
            cadence: cadence_from(options),
            index: options
                .get("device")
                .and_then(|v| v.as_integer())
                .filter(|i| *i >= 0)
                .map(|i| i as u32)
                .unwrap_or(0),
        }
    }
}

/// How a generation ended.
enum Exit {
    Stopped,
    /// Re-initialise after the supervisor-style backoff (nothing was ever published).
    Lost(String),
    /// Re-initialise at once: the generation had published before it failed.
    Healthy(String),
}

fn status(cx: &SourceCtx, state: SourceState, reason: Option<&str>, hint: Option<&str>) {
    cx.status(SourceStatus {
        state,
        reason: reason.map(Arc::from),
        hint: hint.map(Arc::from),
        since: cx.clock.now(),
        last_sample: None,
        dropped: 0,
        restarts: cx.restarts,
    });
}

/// Park on the control channel until stop — the `Mismatch` state.
fn park_forever(cx: &SourceCtx) {
    loop {
        if !cx.sleep_until(cx.next_deadline(IDLE_PARK)) {
            return;
        }
        while cx.try_control().is_some() {}
        if cx.stopped() {
            return;
        }
    }
}

/// A generation that had reached `Ok`/`Degraded` re-initialises without the
/// backoff ladder; one that never published climbs it.
fn lost(state: SourceState, reason: String) -> Exit {
    if matches!(state, SourceState::Ok | SourceState::Degraded) {
        Exit::Healthy(reason)
    } else {
        Exit::Lost(reason)
    }
}

/// Interruptible wait, returning false when stopped.
fn backoff_wait(cx: &SourceCtx, d: Duration) -> bool {
    cx.sleep_until(cx.clock.now().plus(d))
}

impl Source for GpuSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: self.cadence,
            ..demo::gpu_info()
        }
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        status(&cx, SourceState::Starting, None, None);
        let mut backoff = Duration::from_millis(250);
        loop {
            if cx.stopped() {
                return;
            }
            // Backend selection (§11): NVML, else nvidia-smi when only the
            // library is missing, else Unavailable with the reason.
            // `Nvml` lives on this stack frame for the generation; the probe
            // borrows it (and its one `Device` handle) — nothing leaves the
            // thread, nothing is re-fetched per call.
            let nvml = match nvml::init() {
                Ok(n) => Some(n),
                Err(Fail::Mismatch) => {
                    status(
                        &cx,
                        SourceState::Unavailable,
                        Some("driver/library mismatch — reboot"),
                        Some("the NVIDIA driver was upgraded under the running kernel module"),
                    );
                    park_forever(&cx);
                    return;
                }
                Err(Fail::Loading(why)) if smi::available() => {
                    tracing::warn!("NVML unavailable ({why}); nvidia-smi fallback");
                    None
                }
                Err(e) => {
                    status(
                        &cx,
                        SourceState::Unavailable,
                        Some(&format!("NVML: {e}")),
                        Some("is the NVIDIA driver loaded? (`nvidia-smi`)"),
                    );
                    if !backoff_wait(&cx, backoff) {
                        return;
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
            };
            let exit = match &nvml {
                Some(n) => match nvml::NvmlProbe::open(n, self.index) {
                    Ok(mut probe) => self.generation(&cx, &mut probe, None),
                    Err(Fail::Mismatch) => {
                        status(
                            &cx,
                            SourceState::Unavailable,
                            Some("driver/library mismatch — reboot"),
                            None,
                        );
                        park_forever(&cx);
                        return;
                    }
                    Err(e) => Exit::Lost(format!("nvml device {}: {e}", self.index)),
                },
                None => {
                    let mut probe = smi::SmiProbe::new(self.index);
                    self.generation(&cx, &mut probe, Some("nvidia-smi fallback"))
                }
            };
            match exit {
                Exit::Stopped => return,
                Exit::Healthy(reason) => {
                    // A generation that published resets the ladder: a card
                    // lost once a day must not wait 30 s every time.
                    backoff = Duration::from_millis(250);
                    status(
                        &cx,
                        SourceState::Unavailable,
                        Some(&reason),
                        Some("re-initialising"),
                    );
                    if !backoff_wait(&cx, backoff) {
                        return;
                    }
                }
                Exit::Lost(reason) => {
                    status(
                        &cx,
                        SourceState::Unavailable,
                        Some(&reason),
                        Some("re-initialising with backoff"),
                    );
                    if !backoff_wait(&cx, backoff) {
                        return;
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
}

impl GpuSource {
    /// One backend generation: static probe, then the tick loop until stop or
    /// a fatal failure.
    fn generation(&self, cx: &SourceCtx, probe: &mut dyn Probe, degraded: Option<&str>) -> Exit {
        let own_pid = std::process::id();
        let mut poller = Poller::new(DEVICE, own_pid);
        let st = match probe.static_info() {
            Ok(st) => st,
            Err(Fail::Mismatch) => {
                status(
                    cx,
                    SourceState::Unavailable,
                    Some("driver/library mismatch — reboot"),
                    None,
                );
                park_forever(cx);
                return Exit::Stopped;
            }
            Err(e) => return Exit::Lost(format!("{}: {e}", probe.kind())),
        };
        let mut pending_static = Some(poller.static_samples(&st));
        let num_fans = st.num_fans;
        let mut state = SourceState::Starting;
        let mut next_slow = Ts::ZERO;
        let mut next_fans = Ts::ZERO;
        let mut next_procs = Ts::ZERO;
        // Prime the pump (P18): the first tick right away, at whatever the
        // demand is, unless paused.
        let mut first = true;
        loop {
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return Exit::Stopped;
            }
            let level = cx.demand.level();
            let Some(mut period) = self.cadence.for_level(level) else {
                if !cx.sleep_until(cx.next_deadline(IDLE_PARK)) {
                    return Exit::Stopped;
                }
                continue;
            };
            // An idle card in P8 answers the fast tier in ≈ 1.6 ms, not 20 µs
            // (D49): at the focused 250 ms that alone is 6 ms/s. While a fast
            // tick costs over a millisecond the focused tile runs at the
            // visible cadence; under load the card is awake and 250 ms is back.
            if level == Level::Focused && poller.last_fast() > Duration::from_millis(1) {
                period = period.max(self.cadence.visible);
            }
            if !first && !cx.sleep_until(cx.next_deadline(period)) {
                return Exit::Stopped;
            }
            first = false;
            let at = cx.clock.now();
            let level = cx.demand.level();
            let detail = cx.demand.detail();
            let plan = Plan::for_tick(at, level, detail, next_slow, next_fans, next_procs);
            let grid = |p: Duration| Ts(((at.0 / p.as_nanos() as u64) + 1) * p.as_nanos() as u64);
            if plan.slow {
                next_slow = grid(SLOW_PERIOD);
            }
            if plan.fans {
                next_fans = grid(FAN_PERIOD);
            }
            if plan.procs {
                next_procs = grid(PROCS_PERIOD);
            }
            // A table tier that appears mid-grid gets its rows on the next
            // slow tick rather than the next 2 s boundary.
            if detail < gridwatch_store::Detail::Table {
                next_procs = Ts::ZERO;
            }
            match poller.tick(probe, at, plan, num_fans, wall_us(), period) {
                Ok(mut samples) => {
                    if let Some(st) = pending_static.take() {
                        samples.extend(st);
                    }
                    if !samples.is_empty() {
                        cx.emit(at, samples);
                    }
                    let want = if degraded.is_some() {
                        SourceState::Degraded
                    } else {
                        SourceState::Ok
                    };
                    if state != want {
                        state = want;
                        cx.status(SourceStatus {
                            state,
                            reason: degraded.map(Arc::from),
                            hint: None,
                            since: at,
                            last_sample: Some(at),
                            dropped: 0,
                            restarts: cx.restarts,
                        });
                    }
                }
                Err(Fail::Mismatch) => {
                    status(
                        cx,
                        SourceState::Unavailable,
                        Some("driver/library mismatch — reboot"),
                        None,
                    );
                    park_forever(cx);
                    return Exit::Stopped;
                }
                Err(Fail::GpuLost) => return lost(state, "GPU lost".into()),
                Err(e) => return lost(state, format!("{}: {e}", probe.kind())),
            }
        }
    }
}

/// `SourceDef.start` for the registry.
pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(GpuSource::new(options))
}

/// `level` is unused here but part of the cadence contract callers reason
/// about: exposed for tests of the plan.
pub fn fast_period(cadence: &Cadence, level: Level) -> Option<Duration> {
    cadence.for_level(level)
}
