//! pins source gate tests (§8, P14, brief arc 3 seam 3): the sampler over a
//! fake backend — publishing, the lifecycle bridge through a scripted
//! overload, telemetry loss, the redetect counter, `interval_ms` clamping —
//! and an ignored live pass on torch beside the root logger.

use std::time::{Duration, Instant};

use astral_watch::alert::Thresholds;
use astral_watch::config::AlertPolicy;
use astral_watch::decode::{Pin, Reading};
use gridwatch_sources::pins::backend::{Described, Loss, PinsBackend};
use gridwatch_sources::pins::{Bridge, Options, Sampler, clamp_interval};
use gridwatch_store::keys::pins::{PinsMode, PinsState};
use gridwatch_store::{Datum, Sample, Transition, Ts};

fn reading(amps: [f64; 6]) -> Reading {
    let mut pins = [Pin {
        volts: 12.05,
        amps: 0.0,
    }; 6];
    for (p, a) in pins.iter_mut().zip(amps) {
        p.amps = a;
    }
    Reading { pins }
}

/// A scripted chip: answers from a queue of results; counts redetects.
struct Fake {
    queue: Vec<Result<Reading, Loss>>,
    redetects: u32,
    next_bus: u32,
}

impl PinsBackend for Fake {
    fn kind(&self) -> PinsMode {
        PinsMode::I2c
    }
    fn describe(&mut self) -> Result<Described, Loss> {
        Ok(Described {
            bus: Some(self.next_bus),
            addr: 0x2b,
            pci: "0000:01:00.0".into(),
            model: Some("ROG Astral RTX 5090 (variant)".into()),
            access: "unknown".into(),
        })
    }
    fn read(&mut self) -> Result<Reading, Loss> {
        if self.queue.is_empty() {
            Ok(reading([1.7, 1.6, 1.5, 1.5, 1.4, 1.4]))
        } else {
            self.queue.remove(0)
        }
    }
    fn redetect(&mut self) -> Result<bool, Loss> {
        self.redetects += 1;
        self.next_bus += 1;
        Ok(true)
    }
}

fn names(samples: &[Sample]) -> Vec<String> {
    samples.iter().map(|s| s.id.to_string()).collect()
}

fn scalar(samples: &[Sample], name: &str) -> Option<f64> {
    samples.iter().find_map(|s| match &s.datum {
        Datum::Scalar(v) if s.id.to_string() == name => Some(*v),
        _ => None,
    })
}

fn state(samples: &[Sample]) -> PinsState {
    samples
        .iter()
        .find_map(|s| match &s.datum {
            Datum::Record(r) if s.id.name == "pins.state" => {
                r.as_any().downcast_ref::<PinsState>().cloned()
            }
            _ => None,
        })
        .expect("pins.state every sample")
}

fn sampler() -> Sampler {
    Sampler::new(Bridge::new(Thresholds::default(), AlertPolicy::default()))
}

/// One plausible reading publishes the six pins (1-based), the totals, the
/// balance, the read cost and the state.
#[test]
fn a_reading_publishes_the_seam_keys() {
    let mut fake = Fake {
        queue: vec![],
        redetects: 0,
        next_bus: 3,
    };
    let mut s = sampler();
    let t = s.tick(&mut fake, Instant::now(), Ts(500_000_000));
    let n = names(&t.samples);
    assert!(n.contains(&"pins.amps{1}".to_string()) && n.contains(&"pins.amps{6}".to_string()));
    assert!(!n.contains(&"pins.amps{0}".to_string()), "pins are 1-based");
    assert!(n.contains(&"pins.volts{3}".to_string()));
    assert!((scalar(&t.samples, "pins.total_a").unwrap() - 9.1).abs() < 1e-9);
    assert!(scalar(&t.samples, "pins.total_w").unwrap() > 100.0);
    assert!(scalar(&t.samples, "pins.balance").unwrap() > 1.2);
    assert!(scalar(&t.samples, "pins.read_ms").is_some());
    let st = state(&t.samples);
    assert!(!st.telemetry_lost && st.misses == 0 && st.active.is_empty());
    assert!(t.alerts.is_empty() && t.lost.is_none() && !t.redetected);
}

/// The scripted overload through the real lifecycle: raise after three hot
/// samples, resolve after twenty clean ones, the active set travelling in
/// `pins.state` the whole time.
#[test]
fn an_overload_raises_and_resolves_through_the_bridge() {
    let hot = reading([9.5, 9.4, 1.5, 1.5, 1.5, 1.5]);
    let ok = reading([1.7, 1.6, 1.5, 1.5, 1.4, 1.4]);
    let mut queue: Vec<Result<Reading, Loss>> = vec![Ok(hot); 5];
    queue.extend(std::iter::repeat_n(Ok(ok), 25));
    let mut fake = Fake {
        queue,
        redetects: 0,
        next_bus: 3,
    };
    let mut s = sampler();
    let t0 = Instant::now();
    let mut raised_at = None;
    let mut resolved_at = None;
    for i in 0..30u64 {
        let at = Ts((i + 1) * 500_000_000);
        let t = s.tick(&mut fake, t0 + Duration::from_millis(500 * i), at);
        for a in &t.alerts {
            if a.id.0.as_ref() == "pins/overload" {
                match a.transition {
                    Transition::Raised => raised_at = Some(at),
                    Transition::Resolved => resolved_at = Some(at),
                    Transition::Repeated => {}
                }
            }
        }
        let st = state(&t.samples);
        if raised_at.is_some() && resolved_at.is_none() {
            assert!(st.active.iter().any(|c| c.id == "overload"), "tick {i}");
        }
    }
    assert_eq!(raised_at, Some(Ts(3 * 500_000_000)), "3-of-5 confirm");
    // 5 hot + 20 clean = the 25th tick.
    assert_eq!(
        resolved_at,
        Some(Ts(25 * 500_000_000)),
        "20 clean to resolve"
    );
}

/// Loss: no pin keys, `telemetry_lost` in the state, misses counted, a
/// redetect at ten and the counter reset; a permission loss classifies.
#[test]
fn losses_count_misses_and_redetect_at_ten() {
    let mut fake = Fake {
        queue: vec![Err(Loss::Implausible); 12],
        redetects: 0,
        next_bus: 3,
    };
    let mut s = sampler();
    let mut redetected_at = None;
    for i in 0..12u64 {
        let t = s.tick(&mut fake, Instant::now(), Ts((i + 1) * 500_000_000));
        assert!(!names(&t.samples).iter().any(|n| n.starts_with("pins.amps")));
        let st = state(&t.samples);
        assert!(st.telemetry_lost);
        assert_eq!(t.lost, Some(Loss::Implausible));
        if t.redetected {
            redetected_at = Some(i);
            assert_eq!(st.misses, 0, "the counter resets after a redetect");
        }
    }
    assert_eq!(redetected_at, Some(9), "the tenth miss redetects");
    assert_eq!(fake.redetects, 1);
    assert_eq!(
        Loss::Permission.to_string(),
        "permission denied on /dev/i2c-*"
    );
}

#[test]
fn options_clamp_the_interval_and_pick_the_backend() {
    assert_eq!(clamp_interval(100), Duration::from_millis(500), "P14 floor");
    assert_eq!(clamp_interval(700), Duration::from_millis(700));
    assert_eq!(clamp_interval(60_000), Duration::from_secs(5));
    let t: toml::Table = toml::from_str("source = \"exporter\"\ninterval_ms = 250").unwrap();
    let o = Options::from_table(&t);
    assert_eq!(o.pick, gridwatch_sources::pins::Pick::Exporter);
    assert_eq!(o.interval, Duration::from_millis(500));
    assert_eq!(o.exporter, "127.0.0.1:9942");
    let o = Options::from_table(&toml::Table::new());
    assert_eq!(o.pick, gridwatch_sources::pins::Pick::Auto);
}

/// The exporter's staleness rule (digest §2b).
#[test]
fn exporter_judges_up_and_age() {
    use gridwatch_sources::pins::exporter::ExporterBackend;
    use gridwatch_sources::pins::parse::Scrape;
    let r = reading([1.7, 1.6, 1.5, 1.5, 1.4, 1.4]);
    let fresh = Scrape {
        reading: Some(r),
        up: true,
        age_s: Some(0.4),
        active: vec![],
        version: None,
    };
    assert!(ExporterBackend::judge(&fresh, Duration::from_millis(500)).is_ok());
    let down = Scrape {
        up: false,
        ..fresh.clone()
    };
    assert_eq!(
        ExporterBackend::judge(&down, Duration::from_millis(500)),
        Err(Loss::Implausible)
    );
    let old = Scrape {
        age_s: Some(2.0),
        ..fresh
    };
    assert_eq!(
        ExporterBackend::judge(&old, Duration::from_millis(500)),
        Err(Loss::Implausible)
    );
}

/// Live on torch (ignored in CI; **run by hand** — an agent never opens
/// `/dev/i2c-*`, MACHINE.md): P14 — the read cost, transactions per second and
/// the misses over 30 s beside the root `astral-watch log` (PID 6755,
/// bytewise, 0.5 s). Run:
/// `cargo test -p gridwatch-sources --release --test pins live_pins -- --ignored --nocapture`
#[test]
#[ignore = "opens /dev/i2c-*; run by hand on torch"]
fn live_pins_pass_is_inside_p14() {
    use gridwatch_sources::pins::i2c::I2cBackend;
    let mut b = match I2cBackend::detect() {
        Ok(b) => b,
        Err(d) => panic!("detect: {:?} — {}", d, I2cBackend::explain(d).0),
    };
    eprintln!("bus i2c-{} {:?}", b.bus(), b.describe());
    let mut s = sampler();
    let start = Instant::now();
    let (mut reads, mut misses, mut worst) = (0u32, 0u32, 0.0f64);
    let mut sum = 0.0;
    for i in 0..60u64 {
        let t = s.tick(&mut b, Instant::now(), Ts(i * 500_000_000));
        reads += 1;
        sum += s.read_ms;
        worst = worst.max(s.read_ms);
        if t.lost.is_some() {
            misses += 1;
        }
        for a in &t.alerts {
            eprintln!("alert: {} {:?} {}", a.id.0, a.transition, a.detail);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let secs = start.elapsed().as_secs_f64();
    eprintln!(
        "P14: {reads} reads in {secs:.1} s = {:.2} tx/s; read {:.2} ms mean / {worst:.2} ms worst; {misses} misses (TelemetryLost)",
        reads as f64 / secs,
        sum / reads as f64
    );
    assert!(
        reads as f64 / secs <= 2.05,
        "P14: more than 2 transactions/s"
    );
    // The measurable proxy for "block path, one transaction" (digest §2:
    // ≈ 4 ms block vs ≈ 33 ms for the 36-transaction bytewise path).
    assert!(
        sum / reads as f64 <= 12.0,
        "P14: mean read {:.1} ms — the bytewise fallback, not the block read",
        sum / reads as f64
    );
}
