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
    Cadence, Level, OptionIssue, Source, SourceCtx, SourceInfo, SourceState, SourceStatus, Ts, demo,
};

use crate::options::Reader;

pub use poller::{FAN_PERIOD, PROCS_PERIOD, Plan, Poller, SLOW_PERIOD};
pub use probe::{Fail, Probe};

const IDLE_PARK: Duration = Duration::from_secs(5);
const DEVICE: u16 = 0;

/// The option names `[sources.gpu]` owns (§9): `refresh_ms` is the visible
/// fast-tier cadence; `device` picks the NVML index (default 0).
pub const OPTION_NAMES: &[&str] = &["refresh_ms", "device"];
pub const MIN_REFRESH_MS: i64 = 100;
pub const MAX_REFRESH_MS: i64 = 60_000;
/// §9's shipped value, and the default the reader names in a message.
pub const DEFAULT_REFRESH_MS: i64 = 500;

/// What `[sources.gpu]` resolves to (D63).
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub refresh: Duration,
    /// The NVML index.
    pub device: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            refresh: Duration::from_millis(DEFAULT_REFRESH_MS as u64),
            device: u32::from(DEVICE),
        }
    }
}

impl Options {
    pub fn from_table(t: &toml::Table) -> (Options, Vec<OptionIssue>) {
        let mut r = Reader::new("gpu", t);
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
        // A ceiling as well as a floor: `device = 4294967296` used to
        // truncate through `as u32` to 0 — the card the person did not
        // ask for, silently (arc 13 review).
        if let Some(n) = r.int_min("device", 0, "", i64::from(DEVICE)) {
            // A ceiling as well as a floor, and a rejection rather than a
            // clamp: `device = 4294967296` used to truncate through `as u32`
            // to 0 — the card the person did not ask for, silently — and
            // clamping to `u32::MAX` would name a card that cannot exist
            // either (arc 13 review).
            if n > i64::from(u32::MAX) {
                r.rejected(
                    "device",
                    format!(
                        "`device` expects a GPU index, found {n} — \
                         the default {DEVICE} stands"
                    ),
                );
            } else {
                o.device = n as u32;
            }
        }
        o
    }
}

/// The reader `start` runs, without starting anything (§4.3).
pub fn check(t: &toml::Table) -> Vec<OptionIssue> {
    Options::from_table(t).1
}

/// At the shipped 500 ms this is field-identical to `demo::gpu_info().cadence`.
pub fn cadence_from(o: &Options) -> Cadence {
    let visible = o.refresh;
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
        GpuSource::from_options(Options::from_table(options).0)
    }

    pub fn from_options(o: Options) -> GpuSource {
        GpuSource {
            cadence: cadence_from(&o),
            index: o.device,
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
    let (o, issues) = Options::from_table(options);
    crate::options::log("gpu", &issues);
    Box::new(GpuSource::from_options(o))
}

/// `level` is unused here but part of the cadence contract callers reason
/// about: exposed for tests of the plan.
pub fn fast_period(cadence: &Cadence, level: Level) -> Option<Duration> {
    cadence.for_level(level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_store::IssueKind;

    /// The tripwire every other source carries (D63 trap 3): the reader asks
    /// for exactly the keys `OPTION_NAMES` accepts, in order — so a key read
    /// but undeclared, or declared but unread, fails here. `gpu` was the one
    /// source that had no options test at all when arc 13 shipped.
    #[test]
    fn the_reader_asks_for_exactly_the_accepted_set() {
        let t = toml::Table::new();
        let mut r = Reader::new("gpu", &t);
        let o = Options::read(&mut r);
        assert_eq!(r.asked(), OPTION_NAMES);
        assert_eq!(o, Options::default());
        assert!(r.finish().is_empty(), "an empty table is not a problem");
    }

    #[test]
    fn a_wrongly_typed_value_is_rejected_and_the_default_stands() {
        let t: toml::Table = toml::from_str("refresh_ms = \"500\"\ndevice = true").unwrap();
        let (o, issues) = Options::from_table(&t);
        assert_eq!(o, Options::default(), "both defaults stand");
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues.iter().all(|i| i.kind == IssueKind::Rejected));
        assert!(
            issues[0].text.contains("expects an integer"),
            "{:?}",
            issues[0]
        );
        assert!(
            issues[0].text.contains("the default 500 stands"),
            "{:?}",
            issues[0]
        );
    }

    /// The floor and the new ceiling: a negative index and one past `u32`
    /// both leave device 0 rather than selecting a card by accident.
    #[test]
    fn a_device_outside_the_range_never_selects_another_card() {
        for text in ["device = -1", "device = 4294967296"] {
            let t: toml::Table = toml::from_str(text).unwrap();
            let (o, issues) = Options::from_table(&t);
            assert_eq!(o.device, u32::from(DEVICE), "{text}");
            assert_eq!(issues.len(), 1, "{text}: {issues:?}");
        }
    }

    #[test]
    fn a_refresh_out_of_range_is_clamped_and_used() {
        let t: toml::Table = toml::from_str("refresh_ms = 10").unwrap();
        let (o, issues) = Options::from_table(&t);
        assert_eq!(o.refresh, Duration::from_millis(MIN_REFRESH_MS as u64));
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].kind,
            IssueKind::Adjusted,
            "a clamp is used, not discarded"
        );
    }
}
