//! Deterministic pin synthesis (§12.5, brief arc 3 seam 7): six pins around
//! 1.5 A at 12.05 V (≈ 9.2 A / 111 W, balance ≈ 1.3 — torch's own idle
//! numbers), and a **scripted overload**: pins 1 and 2 climb to 9.5 A between
//! `t = 20 s` and `t = 40 s` of the run. The synth emits the lifecycle's
//! `Raised`/`Resolved` events itself at fixed `Ts` — no `Lifecycle`, no
//! `Instant` — so `--demo`, `shot` and the fixture are byte-deterministic.

use std::sync::Arc;
use std::time::Duration;

use crate::alert::{AlertEvent, AlertId, Severity, Transition};
use crate::demo::XorShift;
use crate::key::Datum;
use crate::keys::pins::{self, ActiveCondition, PinsInfo, PinsMode, PinsState};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

/// The overload window on the run clock, and when the scripted lifecycle
/// announces it: raise after the 3-of-5 confirm (three samples at 500 ms),
/// resolve after 20 clean samples (10 s).
pub const OVERLOAD_START_S: f64 = 20.0;
pub const OVERLOAD_END_S: f64 = 40.0;
pub const OVERLOAD_RAISE_S: f64 = 21.5;
pub const OVERLOAD_RESOLVE_S: f64 = 50.0;

/// The synthetic card's static row — torch's Astral on the i2c path.
pub fn pins_info() -> PinsInfo {
    PinsInfo {
        mode: PinsMode::I2c,
        bus: Some(3),
        addr: 0x2b,
        pci: "0000:01:00.0".into(),
        model: Some("ROG Astral RTX 5090 (variant)".into()),
        access: "block".into(),
        interval_ms: 500,
        overload_a: pins::OVERLOAD_A,
        imbalance_ratio: pins::IMBALANCE_RATIO,
        min_load_a: pins::MIN_LOAD_A,
        confirm: 3,
        advisory_confirm: 240,
        resolve: 20,
        repeat_min: 10,
    }
}

#[derive(Clone, Debug)]
pub struct PinsSynth {
    rng: XorShift,
    info_sent: bool,
    raised: bool,
    resolved: bool,
}

/// One tick's output: the batch and any scripted alert events.
pub struct PinsTick {
    pub batch: Batch,
    pub alerts: Vec<AlertEvent>,
}

impl PinsSynth {
    pub fn new(seed: u64) -> PinsSynth {
        PinsSynth {
            rng: XorShift::new(seed.wrapping_add(0x0070_696e)),
            info_sent: false,
            raised: false,
            resolved: false,
        }
    }

    fn overloaded(at: Ts) -> bool {
        let t = at.as_secs_f64();
        (OVERLOAD_START_S..OVERLOAD_END_S).contains(&t)
    }

    /// The six pin currents at `at`: a structurally uneven idle card (the
    /// advisory 1.3× that torch's own CSV shows) and the scripted overload.
    pub fn amps_at(&mut self, at: Ts) -> [f64; 6] {
        let base = [1.72, 1.65, 1.55, 1.50, 1.42, 1.36];
        let mut out = [0.0; 6];
        for (i, b) in base.iter().enumerate() {
            out[i] = (b + self.rng.jitter() * 0.03).max(0.0);
        }
        if Self::overloaded(at) {
            out[0] = 9.5 + self.rng.jitter() * 0.1;
            out[1] = 9.4 + self.rng.jitter() * 0.1;
        }
        out
    }

    pub fn tick_at(&mut self, at: Ts) -> PinsTick {
        let amps = self.amps_at(at);
        let mut samples = Vec::with_capacity(20);
        let mut total_a = 0.0;
        let mut total_w = 0.0;
        let (mut lo, mut hi) = (f64::MAX, 0.0f64);
        for (i, a) in amps.iter().enumerate() {
            let pin = (i + 1) as u16;
            let volts = 12.05 + self.rng.jitter() * 0.02;
            samples.push(Sample {
                id: pins::AMPS.idx(pin).id,
                datum: Datum::Scalar(*a),
            });
            samples.push(Sample {
                id: pins::VOLTS.idx(pin).id,
                datum: Datum::Scalar(volts),
            });
            total_a += a;
            total_w += a * volts;
            lo = lo.min(*a);
            hi = hi.max(*a);
        }
        for (key, v) in [
            (&pins::TOTAL_A, total_a),
            (&pins::TOTAL_W, total_w),
            (&pins::READ_MS, 4.0 + self.rng.f64() * 0.8),
        ] {
            samples.push(Sample {
                id: key.id.clone(),
                datum: Datum::Scalar(v),
            });
        }
        if lo > 0.05 {
            samples.push(Sample {
                id: pins::BALANCE.id.clone(),
                datum: Datum::Scalar(hi / lo),
            });
        }
        if !self.info_sent {
            self.info_sent = true;
            samples.push(Sample {
                id: pins::INFO.id.clone(),
                datum: Datum::Record(Arc::new(pins_info())),
            });
        }
        // The scripted lifecycle: raise at 21.5 s, resolve at 50 s.
        let t = at.as_secs_f64();
        let mut alerts = Vec::new();
        let detail = "OVERLOAD pins 1+2 >9.2A";
        if !self.raised && t >= OVERLOAD_RAISE_S {
            self.raised = true;
            alerts.push(AlertEvent {
                id: AlertId::new(pins::ALERT_OVERLOAD),
                source: pins::SOURCE,
                severity: Severity::Crit,
                transition: Transition::Raised,
                title: Arc::from("OVERLOAD"),
                detail: Arc::from(detail),
                at,
            });
        }
        if self.raised && !self.resolved && t >= OVERLOAD_RESOLVE_S {
            self.resolved = true;
            alerts.push(AlertEvent {
                id: AlertId::new(pins::ALERT_OVERLOAD),
                source: pins::SOURCE,
                severity: Severity::Crit,
                transition: Transition::Resolved,
                title: Arc::from("OVERLOAD"),
                detail: Arc::from("clear after 28s"),
                at,
            });
        }
        let active = if self.raised && !self.resolved {
            vec![ActiveCondition {
                id: "overload".into(),
                detail: detail.into(),
                since: Ts((OVERLOAD_RAISE_S * 1e9) as u64),
            }]
        } else {
            Vec::new()
        };
        samples.push(Sample {
            id: pins::STATE.id.clone(),
            datum: Datum::Record(Arc::new(PinsState {
                telemetry_lost: false,
                misses: 0,
                active,
                service_active: Vec::new(),
            })),
        });
        PinsTick {
            batch: Batch {
                source: pins::SOURCE,
                at,
                samples,
            },
            alerts,
        }
    }
}

pub fn pins_source_info() -> SourceInfo {
    SourceInfo {
        id: pins::SOURCE,
        produces: &["pins.*"],
        cadence: Cadence {
            hidden: Some(Duration::from_secs(1)),
            visible: Duration::from_millis(500),
            focused: Duration::from_millis(500),
            always_on: true,
        },
        // The source probes for itself (exporter or i2c); nothing is required
        // up front, as `probe.rs` already notes for `AstralExporter` (review).
        requires: &[],
    }
}

struct PinsDemoSource {
    seed: u64,
}

impl Source for PinsDemoSource {
    fn info(&self) -> SourceInfo {
        pins_source_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = PinsSynth::new(self.seed);
        let info = self.info();
        let mut interval = info.cadence.visible;
        let at = cx.clock.now();
        let first = synth.tick_at(at);
        cx.emit(at, first.batch.samples);
        for a in first.alerts {
            cx.alert(a);
        }
        cx.status(SourceStatus {
            state: SourceState::Ok,
            reason: Some(Arc::from("synthetic (demo)")),
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        });
        loop {
            // `+`/`−` in `--demo` must do something visible (review): the
            // synth honours `SetOption("interval_ms")` like the live source,
            // clamped to 500–5000 ms, and republishes `pins.info`.
            let mut republish = false;
            while let Some(c) = cx.try_control() {
                if let crate::source::Control::SetOption(k, v) = c
                    && k == "interval_ms"
                    && let Some(ms) = v.as_integer()
                {
                    let new = Duration::from_millis((ms.max(0) as u64).clamp(500, 5000));
                    if new != interval {
                        interval = new;
                        republish = true;
                    }
                }
            }
            if cx.stopped() {
                return;
            }
            // `always_on`: Paused still samples, at the hidden cadence.
            let level = cx.demand.level();
            let cadence = match level {
                crate::source::Level::Paused | crate::source::Level::Hidden => {
                    interval.max(Duration::from_secs(1))
                }
                _ => interval,
            };
            if !cx.sleep_until(cx.next_deadline(cadence)) {
                return;
            }
            let at = cx.clock.now();
            let tick = synth.tick_at(at);
            let mut samples = tick.batch.samples;
            if republish {
                let mut info = pins_info();
                info.interval_ms = interval.as_millis() as u32;
                samples.push(Sample {
                    id: pins::INFO.id.clone(),
                    datum: Datum::Record(Arc::new(info)),
                });
            }
            cx.emit(at, samples);
            for a in tick.alerts {
                cx.alert(a);
            }
        }
    }
}

/// The seeded demo source for `--demo` and `SourceDef.demo` (§4.3).
pub fn pins_demo(seed: u64) -> Box<dyn Source> {
    Box::new(PinsDemoSource { seed })
}
