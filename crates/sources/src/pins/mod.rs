//! The pins source (§5 cadence row, §8, brief arc 3 seam 3): astral-watch's
//! per-pin 12V-2x6 telemetry through one of two backends — the service's
//! exporter when it answers, else the chip over i2c — sampled at 500 ms
//! visible / 1 s hidden and **never stopped** (`always_on`: the alert banner
//! depends on it). The `Lifecycle` debounces; its events leave on the control
//! channel as `pins/<condition>` alerts. Thresholds and policy come from
//! astral-watch's own config, never gridwatch's (D50 §4).

pub mod backend;
pub mod bridge;
pub mod exporter;
pub mod i2c;
pub mod parse;

use std::sync::Arc;
use std::time::{Duration, Instant};

use astral_watch::config::{AlertPolicy, Config};
use astral_watch::i2c::REDETECT_AFTER;
use gridwatch_store::keys::pins::{self, PinsInfo, PinsMode, PinsState};
use gridwatch_store::{
    Cadence, Control, Datum, Sample, Source, SourceCtx, SourceInfo, SourceState, SourceStatus, Ts,
    demo,
};

pub use backend::{Described, Loss, PinsBackend};
pub use bridge::Bridge;

/// `[sources.pins]` (§9): the backend choice, the exporter address and the
/// sample interval (clamped 500–5000 ms — P14).
pub const OPTION_NAMES: &[&str] = &["source", "exporter", "interval_ms"];
pub const DEFAULT_EXPORTER: &str = "127.0.0.1:9942";
pub const MIN_INTERVAL: Duration = Duration::from_millis(500);
pub const MAX_INTERVAL: Duration = Duration::from_secs(5);
/// How often an unavailable source, or a chosen exporter, is re-probed.
pub const REPROBE: Duration = Duration::from_secs(10);
/// Misses before the status turns `Degraded`.
const DEGRADED_AFTER: u32 = 3;

/// `gridwatch doctor`'s live probes (seam 10): the exporter is asked once
/// (250 ms connect), then `detect_bus` — which opens `/dev/i2c-*`, so this
/// never runs at startup and never from a test on torch (MACHINE.md).
pub fn doctor(exporter: Option<&str>) -> Vec<(gridwatch_store::Capability, bool, String)> {
    use gridwatch_store::Capability;
    let addr = exporter.unwrap_or(DEFAULT_EXPORTER);
    let mut out = Vec::new();
    match exporter::fetch(addr) {
        Ok(body) => {
            let scrape = parse::parse_metrics(&body);
            let version = scrape
                .version
                .as_deref()
                .map(|v| format!(" (astral-watch {v})"))
                .unwrap_or_default();
            out.push((
                Capability::AstralExporter,
                true,
                format!("answers at {addr}{version}"),
            ));
        }
        Err(e) => out.push((
            Capability::AstralExporter,
            false,
            format!("no answer at {addr}: {e}"),
        )),
    }
    match i2c::I2cBackend::detect() {
        Ok(b) => out.push((
            Capability::I2cNvidia,
            true,
            format!("chip found on i2c-{} @ 0x2b", b.bus()),
        )),
        Err(d) => {
            let (reason, hint) = i2c::I2cBackend::explain(d);
            out.push((Capability::I2cNvidia, false, format!("{reason} — {hint}")));
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Auto,
    I2c,
    Exporter,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub pick: Pick,
    pub exporter: String,
    pub interval: Duration,
}

pub fn clamp_interval(ms: i64) -> Duration {
    Duration::from_millis((ms.max(0) as u64).clamp(
        MIN_INTERVAL.as_millis() as u64,
        MAX_INTERVAL.as_millis() as u64,
    ))
}

impl Options {
    pub fn from_table(t: &toml::Table) -> Options {
        let pick = match t.get("source").and_then(|v| v.as_str()) {
            Some("i2c") => Pick::I2c,
            Some("exporter") => Pick::Exporter,
            _ => Pick::Auto,
        };
        Options {
            pick,
            exporter: t
                .get("exporter")
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_EXPORTER)
                .to_string(),
            interval: t
                .get("interval_ms")
                .and_then(|v| v.as_integer())
                .map(|ms| {
                    let c = clamp_interval(ms);
                    if c.as_millis() as i64 != ms {
                        tracing::warn!(
                            "[sources.pins] interval_ms = {ms} clamped to {} (P14: 500–5000)",
                            c.as_millis()
                        );
                    }
                    c
                })
                .unwrap_or(MIN_INTERVAL),
        }
    }
}

pub struct PinsSource {
    options: Options,
}

impl PinsSource {
    pub fn new(options: &toml::Table) -> PinsSource {
        PinsSource {
            options: Options::from_table(options),
        }
    }

    fn cadence(&self) -> Cadence {
        Cadence {
            hidden: Some(self.options.interval.max(Duration::from_secs(1))),
            visible: self.options.interval,
            focused: self.options.interval,
            always_on: true,
        }
    }
}

/// The samples one plausible reading publishes.
pub fn reading_samples(r: &astral_watch::decode::Reading, read_ms: f64) -> Vec<Sample> {
    let mut out = Vec::with_capacity(16);
    for (i, p) in r.pins.iter().enumerate() {
        let pin = (i + 1) as u16;
        out.push(Sample {
            id: pins::AMPS.idx(pin).id,
            datum: Datum::Scalar(p.amps),
        });
        out.push(Sample {
            id: pins::VOLTS.idx(pin).id,
            datum: Datum::Scalar(p.volts),
        });
    }
    for (key, v) in [
        (&pins::TOTAL_A, r.total_amps()),
        (&pins::TOTAL_W, r.total_watts()),
        (&pins::READ_MS, read_ms),
    ] {
        out.push(Sample {
            id: key.id.clone(),
            datum: Datum::Scalar(v),
        });
    }
    if let Some(b) = r.balance() {
        out.push(Sample {
            id: pins::BALANCE.id.clone(),
            datum: Datum::Scalar(b),
        });
    }
    out
}

pub fn info_sample(
    d: &Described,
    mode: PinsMode,
    interval: Duration,
    thresholds: &astral_watch::alert::Thresholds,
    policy: &AlertPolicy,
) -> Sample {
    Sample {
        id: pins::INFO.id.clone(),
        datum: Datum::Record(Arc::new(PinsInfo {
            mode,
            bus: d.bus,
            addr: d.addr,
            pci: d.pci.clone(),
            model: d.model.clone(),
            access: d.access.clone(),
            interval_ms: interval.as_millis() as u32,
            overload_a: thresholds.overload_amps,
            imbalance_ratio: thresholds.imbalance_ratio,
            min_load_a: thresholds.min_load_amps,
            confirm: policy.confirm_samples,
            advisory_confirm: policy.advisory_confirm_samples,
            resolve: policy.resolve_samples,
            repeat_min: policy.repeat_minutes as u32,
        })),
    }
}

pub fn state_sample(bridge: &Bridge, lost: bool, misses: u32, service: Vec<String>) -> Sample {
    Sample {
        id: pins::STATE.id.clone(),
        datum: Datum::Record(Arc::new(PinsState {
            telemetry_lost: lost,
            misses,
            active: bridge.active().to_vec(),
            service_active: service,
        })),
    }
}

/// One tick over any backend: read, publish, debounce, alert. Pure over its
/// inputs but for the `Instant` the lifecycle wants; tested with a fake.
pub struct Sampler {
    pub bridge: Bridge,
    pub misses: u32,
    pub read_ms: f64,
}

pub struct Ticked {
    pub samples: Vec<Sample>,
    pub alerts: Vec<gridwatch_store::AlertEvent>,
    pub lost: Option<Loss>,
    /// The backend moved (redetect found a new bus): re-describe.
    pub redetected: bool,
}

impl Sampler {
    pub fn new(bridge: Bridge) -> Sampler {
        Sampler {
            bridge,
            misses: 0,
            read_ms: 0.0,
        }
    }

    pub fn tick(&mut self, backend: &mut dyn PinsBackend, now: Instant, at: Ts) -> Ticked {
        let t0 = Instant::now();
        let read = backend.read();
        self.read_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut redetected = false;
        let (mut samples, alerts, lost) = match read {
            Ok(r) => {
                self.misses = 0;
                let alerts = self.bridge.observe(now, at, Ok(&r));
                (reading_samples(&r, self.read_ms), alerts, None)
            }
            Err(loss) => {
                self.misses += 1;
                let msg = loss.to_string();
                let alerts = self.bridge.observe(now, at, Err(&msg));
                let mut loss = loss;
                if self.misses >= REDETECT_AFTER {
                    self.misses = 0;
                    match backend.redetect() {
                        Ok(moved) => redetected = moved,
                        // The card is gone for good: say so rather than
                        // looping on "GPU idle?" (review).
                        Err(e) => loss = e,
                    }
                }
                (Vec::new(), alerts, Some(loss))
            }
        };
        samples.push(state_sample(
            &self.bridge,
            lost.is_some(),
            self.misses,
            backend.service_active(),
        ));
        Ticked {
            samples,
            alerts,
            lost,
            redetected,
        }
    }
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

/// Pick a backend per the options (brief seam 3): exporter if it answers,
/// then i2c if the chip is found, else the reason.
fn choose(opts: &Options) -> Result<Box<dyn PinsBackend>, (String, String)> {
    let try_exporter = || -> Option<Box<dyn PinsBackend>> {
        exporter::ExporterBackend::reachable(&opts.exporter).then(|| {
            Box::new(exporter::ExporterBackend::new(
                &opts.exporter,
                opts.interval,
            )) as _
        })
    };
    let try_i2c = || -> Result<Box<dyn PinsBackend>, (String, String)> {
        match i2c::I2cBackend::detect() {
            Ok(b) => Ok(Box::new(b)),
            Err(d) => {
                let (reason, hint) = i2c::I2cBackend::explain(d);
                Err((reason.to_string(), hint.to_string()))
            }
        }
    };
    match opts.pick {
        Pick::Exporter => try_exporter().ok_or_else(|| {
            (
                format!("exporter {} does not answer", opts.exporter),
                "start `astral-watch` with `[export]` enabled, or set `[sources.pins] source = \"i2c\"`".into(),
            )
        }),
        Pick::I2c => try_i2c(),
        Pick::Auto => match try_exporter() {
            Some(b) => Ok(b),
            None => try_i2c(),
        },
    }
}

impl Source for PinsSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: self.cadence(),
            ..demo::pins_source_info()
        }
    }

    fn run(mut self: Box<Self>, cx: SourceCtx) {
        status(&cx, SourceState::Starting, None, None);
        // astral-watch's own thresholds and policy — never gridwatch's (D50 §4).
        let (config, path) = astral_watch::config::load(None).unwrap_or_else(|e| {
            tracing::warn!("astral-watch config: {e}; using defaults");
            (Config::default(), None)
        });
        if let Some(p) = path {
            tracing::info!("astral-watch thresholds from {}", p.display());
        }
        for w in config.warnings() {
            tracing::warn!("astral-watch config: {w}");
        }
        // One lifecycle for the whole run: backends come and go underneath it,
        // so an alert raised on one generation resolves on the next instead of
        // being orphaned in the store (review).
        let mut sampler = Sampler::new(Bridge::new(config.thresholds, config.alerts));
        loop {
            if cx.stopped() {
                return;
            }
            let mut backend = match choose(&self.options) {
                Ok(b) => b,
                Err((reason, hint)) => {
                    status(&cx, SourceState::Unavailable, Some(&reason), Some(&hint));
                    // No telemetry at all is a `TelemetryLost` sample too,
                    // so the lifecycle freezes what was active.
                    let at = cx.clock.now();
                    let alerts = sampler.bridge.observe(Instant::now(), at, Err(&reason));
                    cx.emit(at, vec![state_sample(&sampler.bridge, true, 0, Vec::new())]);
                    for a in alerts {
                        cx.alert(a);
                    }
                    if !cx.sleep_until(cx.clock.now().plus(REPROBE)) {
                        return;
                    }
                    while let Some(c) = cx.try_control() {
                        self.apply_control(c);
                    }
                    continue;
                }
            };
            sampler.misses = 0;
            let mut described = backend.describe().unwrap_or_default();
            let mode = backend.kind();
            let mut pending_info = Some(info_sample(
                &described,
                mode,
                self.options.interval,
                sampler.bridge.thresholds(),
                &config.alerts,
            ));
            let mut state = SourceState::Starting;
            let mut last_reprobe = Instant::now();
            let mut first = true;
            let mut last_loss: Option<Loss> = None;
            loop {
                let mut interval_changed = false;
                while let Some(c) = cx.try_control() {
                    interval_changed |= self.apply_control(c);
                }
                if cx.stopped() {
                    return;
                }
                if interval_changed {
                    backend.set_interval(self.options.interval);
                    pending_info = Some(info_sample(
                        &described,
                        mode,
                        self.options.interval,
                        sampler.bridge.thresholds(),
                        &config.alerts,
                    ));
                }
                let cadence = self.cadence();
                // `always_on`: Paused samples at the hidden cadence, never None.
                let mut period = cadence
                    .for_level(cx.demand.level())
                    .unwrap_or(cadence.visible);
                // A deeply idle GPU answers zeros: astral-watch re-probes
                // bytewise (36 transactions) on every implausible reading, so
                // while the chip keeps answering implausibly the source backs
                // off to 5 s (P14, review) and returns on the first good one.
                if sampler.misses >= DEGRADED_AFTER && matches!(last_loss, Some(Loss::Implausible))
                {
                    period = MAX_INTERVAL;
                }
                if !first && !cx.sleep_until(cx.next_deadline(period)) {
                    return;
                }
                first = false;
                let at = cx.clock.now();
                let ticked = sampler.tick(backend.as_mut(), Instant::now(), at);
                if ticked.redetected {
                    described = backend.describe().unwrap_or_default();
                    pending_info = Some(info_sample(
                        &described,
                        mode,
                        self.options.interval,
                        sampler.bridge.thresholds(),
                        &config.alerts,
                    ));
                }
                last_loss = ticked.lost.clone();
                let mut samples = ticked.samples;
                if let Some(info) = pending_info.take() {
                    samples.push(info);
                }
                cx.emit(at, samples);
                for a in ticked.alerts {
                    cx.alert(a);
                }
                let want = match &ticked.lost {
                    None => SourceState::Ok,
                    Some(_) if sampler.misses >= DEGRADED_AFTER || sampler.misses == 0 => {
                        SourceState::Degraded
                    }
                    Some(_) => state,
                };
                if want != state {
                    state = want;
                    let (reason, hint) = match &ticked.lost {
                        None => (None, None),
                        Some(Loss::Permission) => (
                            Some("permission denied on /dev/i2c-*"),
                            Some("add yourself to the i2c group"),
                        ),
                        Some(_) => (
                            Some("waiting for telemetry (GPU idle?)"),
                            Some("the chip answers zeros while the GPU is deeply idle"),
                        ),
                    };
                    cx.status(SourceStatus {
                        state,
                        reason: reason.map(Arc::from),
                        hint: hint.map(Arc::from),
                        since: at,
                        last_sample: Some(at),
                        dropped: 0,
                        restarts: cx.restarts,
                    });
                }
                // A dead exporter, or a permission loss, hands over to a new
                // generation on the next probe.
                if matches!(ticked.lost, Some(Loss::NotFound) | Some(Loss::Permission)) {
                    break;
                }
                // In `auto`, a chosen i2c backend re-checks for the exporter
                // every REPROBE so the service takes over when it appears.
                if self.options.pick == Pick::Auto
                    && mode == PinsMode::I2c
                    && last_reprobe.elapsed() >= REPROBE
                {
                    last_reprobe = Instant::now();
                    if exporter::ExporterBackend::reachable(&self.options.exporter) {
                        break;
                    }
                }
            }
        }
    }
}

impl PinsSource {
    /// `SetOption("interval_ms", n)` is the one live option (seam 3); true when
    /// the interval changed.
    fn apply_control(&mut self, c: Control) -> bool {
        match c {
            Control::SetOption(k, v) if k == "interval_ms" => {
                if let Some(ms) = v.as_integer() {
                    let new = clamp_interval(ms);
                    if new != self.options.interval {
                        self.options.interval = new;
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

/// `SourceDef.start` for the registry.
pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(PinsSource::new(options))
}
