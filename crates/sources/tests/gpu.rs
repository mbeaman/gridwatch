//! gpu source gate tests (§8, P11–P13, brief 2b): the tier logic over a fake
//! probe — pruning, the PCIe diff, the utilisation `last_seen`, the process
//! gate, `InsufficientSize` — plus an ignored live pass on torch that records
//! P11's per-class numbers.

use std::collections::BTreeSet;
use std::time::Duration;

use gridwatch_sources::gpu::poller::{Class, Field, Plan, Poller, counter_delta};
use gridwatch_sources::gpu::probe::{Fail, Probe, ProcMem, ProcUtil, Static};
use gridwatch_store::keys::gpu::{self, GpuProcKind, GpuProcs, Throttle};
use gridwatch_store::{Datum, Detail, Label, Level, Sample, Ts};

/// A scripted backend: every field answers from a table, calls are counted.
#[derive(Default)]
struct Fake {
    calls: Vec<&'static str>,
    pcie: (u64, u64),
    proc_fail: Option<Fail>,
    util_samples: Vec<ProcUtil>,
    fans_unsupported: bool,
    memory_temp_field_calls: u32,
}

impl Fake {
    fn count(&self, name: &str) -> usize {
        self.calls.iter().filter(|c| **c == name).count()
    }
}

impl Probe for Fake {
    fn kind(&self) -> &'static str {
        "fake"
    }
    fn static_info(&mut self) -> Result<Static, Fail> {
        Ok(Static {
            name: "NVIDIA GeForce RTX 5090".into(),
            pci_id: 0x2B85,
            cores: Some(21760),
            bus_width: Some(512),
            num_fans: 3,
            clock_gfx_max_mhz: Some(3135),
            temp_slowdown_c: Some(93),
            ..Static::default()
        })
    }
    fn utilization(&mut self) -> Result<(u32, u32), Fail> {
        self.calls.push("util");
        Ok((19, 5))
    }
    fn temperature_c(&mut self) -> Result<u32, Fail> {
        self.calls.push("temp");
        Ok(45)
    }
    fn power_w(&mut self) -> Result<f64, Fail> {
        self.calls.push("power");
        Ok(108.2)
    }
    fn power_limit_w(&mut self) -> Result<f64, Fail> {
        Ok(600.0)
    }
    fn clock_gfx_mhz(&mut self) -> Result<u32, Fail> {
        Ok(2220)
    }
    fn clock_mem_mhz(&mut self) -> Result<u32, Fail> {
        Ok(7001)
    }
    fn pstate(&mut self) -> Result<u8, Fail> {
        Ok(3)
    }
    fn throttle_bits(&mut self) -> Result<u64, Fail> {
        Ok(Throttle::SW_POWER_CAP)
    }
    fn memory_b(&mut self) -> Result<(u64, u64), Fail> {
        self.calls.push("memory");
        Ok((13856 << 20, 32607 << 20))
    }
    fn encoder_pct(&mut self) -> Result<u32, Fail> {
        // The card that says NotSupported once must never be asked again.
        self.memory_temp_field_calls += 1;
        Err(Fail::NotSupported)
    }
    fn decoder_pct(&mut self) -> Result<u32, Fail> {
        Ok(0)
    }
    fn pcie_link(&mut self) -> Result<(u32, u32), Fail> {
        Ok((5, 16))
    }
    fn pcie_bytes(&mut self) -> Result<(u64, u64), Fail> {
        self.calls.push("pcie");
        self.pcie.0 += 2_000_000;
        self.pcie.1 += 1_000_000;
        Ok(self.pcie)
    }
    fn fan_pct(&mut self, fan: u32) -> Result<u32, Fail> {
        self.calls.push("fan");
        if self.fans_unsupported {
            return Err(Fail::NotSupported);
        }
        Ok(30 + fan)
    }
    fn fan_rpm(&mut self, _fan: u32) -> Result<u32, Fail> {
        self.calls.push("rpm");
        Ok(514)
    }
    fn power_samples(&mut self, last_ts: u64) -> Result<Vec<(u64, f32)>, Fail> {
        self.calls.push("samples");
        Ok(vec![(last_ts + 20_000, 100.0), (last_ts + 40_000, 110.0)])
    }
    fn graphics_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
        self.calls.push("gprocs");
        if let Some(f) = &self.proc_fail {
            return Err(f.clone());
        }
        Ok(vec![
            ProcMem {
                pid: 1701,
                vram_b: Some(464 << 20),
            },
            ProcMem {
                pid: 412345,
                vram_b: Some(12579 << 20),
            },
            ProcMem {
                pid: std::process::id(),
                vram_b: Some(1 << 20),
            },
        ])
    }
    fn compute_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
        self.calls.push("cprocs");
        Ok(vec![
            ProcMem {
                pid: 412345,
                vram_b: Some(12579 << 20),
            },
            ProcMem {
                pid: 11805,
                vram_b: Some(44 << 20),
            },
        ])
    }
    fn proc_util(&mut self, last_seen_us: u64) -> Result<Vec<ProcUtil>, Fail> {
        self.calls.push("putil");
        let fresh: Vec<ProcUtil> = self
            .util_samples
            .iter()
            .copied()
            .filter(|s| s.timestamp_us > last_seen_us)
            .collect();
        if fresh.is_empty() {
            Err(Fail::NotFound)
        } else {
            Ok(fresh)
        }
    }
}

fn names(samples: &[Sample]) -> BTreeSet<String> {
    samples.iter().map(|s| s.id.to_string()).collect()
}

fn scalar(samples: &[Sample], name: &str) -> Option<f64> {
    samples
        .iter()
        .find_map(|s| match (&s.datum, s.id.to_string() == name) {
            (Datum::Scalar(v), true) => Some(*v),
            _ => None,
        })
}

fn procs(samples: &[Sample]) -> Option<GpuProcs> {
    samples.iter().find_map(|s| match &s.datum {
        Datum::Record(r) if s.id.name == "gpu.procs" => {
            r.as_any().downcast_ref::<GpuProcs>().cloned()
        }
        _ => None,
    })
}

const TICK: Duration = Duration::from_millis(500);
const WALL: u64 = 1_756_700_000_000_000;

fn plan(slow: bool, fans: bool, trace: bool, procs: bool) -> Plan {
    Plan {
        slow,
        fans,
        power_trace: trace,
        procs,
    }
}

/// P13: the fast tier alone publishes only the fast keys; the slow tier adds
/// memory/enc/dec/PCIe; process rows appear only when the plan asks.
#[test]
fn tiers_publish_their_own_keys_and_nothing_more() {
    let mut fake = Fake::default();
    let mut p = Poller::new(0, std::process::id());
    let fast = p
        .tick(
            &mut fake,
            Ts(0),
            plan(false, false, false, false),
            3,
            WALL,
            TICK,
        )
        .unwrap();
    let n = names(&fast);
    assert!(n.contains("gpu.util_pct{0}") && n.contains("gpu.memctl_pct{0}"));
    assert!(n.contains("gpu.power_w{0}") && n.contains("gpu.throttle{0}"));
    assert!(
        !n.contains("gpu.vram_used_b{0}") && !n.contains("gpu.procs{0}"),
        "fast tier leaked slow keys: {n:?}"
    );
    assert_eq!(fake.count("memory"), 0);
    let slow = p
        .tick(
            &mut fake,
            Ts(2_000_000_000),
            plan(true, false, false, false),
            3,
            WALL,
            TICK,
        )
        .unwrap();
    let n = names(&slow);
    assert!(n.contains("gpu.vram_used_b{0}") && n.contains("gpu.dec_pct{0}"));
    assert!(n.contains("gpu.pcie_gen{0}"));
    assert!(
        !n.contains("gpu.procs{0}"),
        "no procs without Detail::Table"
    );
    assert!(
        !n.contains("gpu.fan_pct{0:0}"),
        "fans only on their 5 s grid"
    );
    assert!(
        !n.contains("gpu.power_trace{0}"),
        "trace only while visible"
    );
    assert_eq!(fake.count("gprocs") + fake.count("putil"), 0, "P13");
    // The ms/s evidence rides the slow tier, one label per class.
    assert!(n.contains("gpu.nvml_ms{fast}") && n.contains("gpu.nvml_ms{procs}"));
}

/// §8 pruning: a `NotSupported` field is asked exactly once.
#[test]
fn not_supported_is_pruned_after_one_call() {
    let mut fake = Fake {
        fans_unsupported: true,
        ..Fake::default()
    };
    let mut p = Poller::new(0, 1);
    for i in 0..4u64 {
        let out = p
            .tick(
                &mut fake,
                Ts(i * 1_000_000_000),
                plan(true, true, false, false),
                3,
                WALL,
                TICK,
            )
            .unwrap();
        assert!(!names(&out).contains("gpu.enc_pct{0}"));
    }
    assert_eq!(
        fake.memory_temp_field_calls, 1,
        "encoder asked more than once"
    );
    assert!(p.is_pruned(Field::Encoder));
    // Each of the three fans was asked once for %, and RPM (supported) every time.
    assert_eq!(fake.count("fan"), 3, "fan % pruned per fan");
    assert_eq!(fake.count("rpm"), 12);
}

/// PCIe: byte counters diffed to B/s over the real interval; nothing on the
/// first sample.
#[test]
fn pcie_counters_are_diffed_to_bytes_per_second() {
    let mut fake = Fake::default();
    let mut p = Poller::new(0, 1);
    let first = p
        .tick(
            &mut fake,
            Ts(0),
            plan(true, false, false, false),
            0,
            WALL,
            TICK,
        )
        .unwrap();
    assert!(scalar(&first, "gpu.pcie_tx_bps{0}").is_none());
    let second = p
        .tick(
            &mut fake,
            Ts(2_000_000_000),
            plan(true, false, false, false),
            0,
            WALL,
            TICK,
        )
        .unwrap();
    assert_eq!(scalar(&second, "gpu.pcie_tx_bps{0}"), Some(1_000_000.0));
    assert_eq!(scalar(&second, "gpu.pcie_rx_bps{0}"), Some(500_000.0));
}

/// The process rows: own pid filtered (P12), `Both` merged, `last_seen`
/// carried forward so a stale sample never re-applies, `NotFound` = zeros.
#[test]
fn process_rows_merge_filter_and_carry_last_seen_forward() {
    let mut fake = Fake {
        util_samples: vec![ProcUtil {
            pid: 412345,
            timestamp_us: WALL - 100_000,
            sm: 17,
            mem: 9,
            enc: 0,
            dec: 0,
        }],
        ..Fake::default()
    };
    let mut p = Poller::new(0, std::process::id());
    let out = p
        .tick(
            &mut fake,
            Ts(0),
            plan(true, false, false, true),
            0,
            WALL,
            TICK,
        )
        .unwrap();
    let t = procs(&out).expect("gpu.procs");
    let pids: Vec<i32> = t.rows.iter().map(|r| r.pid).collect();
    assert_eq!(pids, vec![1701, 11805, 412345], "own pid is gone (P12)");
    let game = t.rows.iter().find(|r| r.pid == 412345).unwrap();
    assert_eq!(game.kind, GpuProcKind::Both);
    assert_eq!((game.sm_pct, game.fresh), (17, true));
    assert_eq!(t.vram_total_b, 32607 << 20);
    // Second tick: the only sample is older than last_seen → NotFound → zeros.
    let out = p
        .tick(
            &mut fake,
            Ts(1_000_000_000),
            plan(true, false, false, true),
            0,
            WALL + 1_000_000,
            TICK,
        )
        .unwrap();
    let t = procs(&out).unwrap();
    let game = t.rows.iter().find(|r| r.pid == 412345).unwrap();
    assert_eq!((game.sm_pct, game.fresh), (0, false));
}

/// `InsufficientSize` on a list keeps the previous rows; `NotSupported` on
/// one list is not the end of the table.
#[test]
fn insufficient_size_keeps_the_previous_rows() {
    let mut fake = Fake::default();
    let mut p = Poller::new(0, std::process::id());
    let out = p
        .tick(
            &mut fake,
            Ts(0),
            plan(true, false, false, true),
            0,
            WALL,
            TICK,
        )
        .unwrap();
    assert_eq!(procs(&out).unwrap().rows.len(), 3);
    fake.proc_fail = Some(Fail::InsufficientSize);
    let out = p
        .tick(
            &mut fake,
            Ts(1_000_000_000),
            plan(true, false, false, true),
            0,
            WALL,
            TICK,
        )
        .unwrap();
    assert_eq!(procs(&out).unwrap().rows.len(), 3, "previous rows kept");
    fake.proc_fail = Some(Fail::NotSupported);
    let out = p
        .tick(
            &mut fake,
            Ts(2_000_000_000),
            plan(true, false, false, true),
            0,
            WALL,
            TICK,
        )
        .unwrap();
    let rows = procs(&out).unwrap().rows;
    assert_eq!(rows.len(), 2, "compute list alone: {rows:?}");
    assert!(rows.iter().all(|r| r.kind == GpuProcKind::Compute));
}

/// Fatal failures abort the tick with the reason the source acts on.
#[test]
fn gpu_lost_is_fatal_for_the_tick() {
    struct Lost;
    impl Probe for Lost {
        fn kind(&self) -> &'static str {
            "lost"
        }
        fn static_info(&mut self) -> Result<Static, Fail> {
            Err(Fail::GpuLost)
        }
        fn utilization(&mut self) -> Result<(u32, u32), Fail> {
            Err(Fail::GpuLost)
        }
        fn temperature_c(&mut self) -> Result<u32, Fail> {
            unreachable!("the tick stops at the first fatal call")
        }
        fn power_w(&mut self) -> Result<f64, Fail> {
            unreachable!()
        }
        fn power_limit_w(&mut self) -> Result<f64, Fail> {
            unreachable!()
        }
        fn clock_gfx_mhz(&mut self) -> Result<u32, Fail> {
            unreachable!()
        }
        fn clock_mem_mhz(&mut self) -> Result<u32, Fail> {
            unreachable!()
        }
        fn pstate(&mut self) -> Result<u8, Fail> {
            unreachable!()
        }
        fn throttle_bits(&mut self) -> Result<u64, Fail> {
            unreachable!()
        }
        fn memory_b(&mut self) -> Result<(u64, u64), Fail> {
            unreachable!()
        }
        fn encoder_pct(&mut self) -> Result<u32, Fail> {
            unreachable!()
        }
        fn decoder_pct(&mut self) -> Result<u32, Fail> {
            unreachable!()
        }
        fn pcie_link(&mut self) -> Result<(u32, u32), Fail> {
            unreachable!()
        }
        fn pcie_bytes(&mut self) -> Result<(u64, u64), Fail> {
            unreachable!()
        }
        fn fan_pct(&mut self, _: u32) -> Result<u32, Fail> {
            unreachable!()
        }
        fn fan_rpm(&mut self, _: u32) -> Result<u32, Fail> {
            unreachable!()
        }
        fn power_samples(&mut self, _: u64) -> Result<Vec<(u64, f32)>, Fail> {
            unreachable!()
        }
        fn graphics_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
            unreachable!()
        }
        fn compute_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
            unreachable!()
        }
        fn proc_util(&mut self, _: u64) -> Result<Vec<ProcUtil>, Fail> {
            unreachable!()
        }
    }
    let mut p = Poller::new(0, 1);
    let r = p.tick(
        &mut Lost,
        Ts(0),
        plan(true, true, true, true),
        3,
        WALL,
        TICK,
    );
    assert_eq!(r.unwrap_err(), Fail::GpuLost);
}

/// The plan: slow on its grid, fans on theirs, the trace while visible, the
/// process rows at `Detail::Table` (D49 on the trace gate).
#[test]
fn the_plan_follows_level_detail_and_the_grids() {
    let at = Ts(5_000_000_000);
    let p = Plan::for_tick(at, Level::Hidden, Detail::Meters, at, at, at);
    assert!(p.slow && p.fans && !p.power_trace && !p.procs);
    let p = Plan::for_tick(at, Level::Visible, Detail::Table, at, Ts(9_000_000_000), at);
    assert!(p.slow && !p.fans && p.power_trace && p.procs);
    let p = Plan::for_tick(at, Level::Visible, Detail::Table, at, at, Ts(6_000_000_000));
    assert!(p.slow && !p.procs, "process rows keep their own 2 s grid");
    let p = Plan::for_tick(at, Level::Focused, Detail::Table, Ts(6_000_000_000), at, at);
    assert!(
        !p.slow && !p.fans && !p.power_trace && !p.procs,
        "nothing slow off-grid"
    );
    assert_eq!(Class::Procs.label(), "procs");
    // The PCIe byte counters are 32-bit on driver 610 (review): a wrap is a
    // step, never a zero.
    assert_eq!(counter_delta(100, 150), 50);
    assert_eq!(counter_delta((1u64 << 32) - 10, 5), 15);
    assert_eq!(
        counter_delta(1u64 << 40, 5),
        0,
        "a 64-bit counter never wraps here"
    );
}

/// The static samples: `gpu.info` with the spec row cross-checked, the max
/// clocks and slowdown threshold as scalars.
#[test]
fn static_samples_carry_the_info_record_and_the_spec_row() {
    let mut fake = Fake::default();
    let st = fake.static_info().unwrap();
    let p = Poller::new(0, 1);
    let out = p.static_samples(&st);
    let n = names(&out);
    assert!(n.contains("gpu.info{0}") && n.contains("gpu.clock_gfx_max_mhz{0}"));
    assert_eq!(scalar(&out, "gpu.temp_slowdown_c{0}"), Some(93.0));
    let info = out
        .iter()
        .find_map(|s| match &s.datum {
            Datum::Record(r) => r.as_any().downcast_ref::<gpu::GpuInfo>().cloned(),
            _ => None,
        })
        .unwrap();
    assert_eq!(info.spec.as_ref().map(|s| s.sms), Some(170));
    assert!(!info.spec_mismatch);
    let _ = Label::None;
}

/// Live on torch (ignored in CI): P11's per-class ms/s with process rows on,
/// P12's own-pid filter, and the pruning list. Run:
/// `cargo test -p gridwatch-sources --release --test gpu -- --ignored --nocapture`
#[test]
#[ignore = "needs the NVIDIA driver; run by hand on torch"]
fn live_nvml_pass_is_inside_p11() {
    use gridwatch_sources::gpu::nvml::{NvmlProbe, init};
    let nvml = init().expect("NVML");
    let mut probe = NvmlProbe::open(&nvml, 0).expect("device 0");
    let st = probe.static_info().unwrap();
    eprintln!("{st:#?}");
    let mut p = Poller::new(0, std::process::id());
    let start = std::time::Instant::now();
    let mut last = None;
    let (mut next_slow, mut next_fans, mut next_procs) = (Ts::ZERO, Ts::ZERO, Ts::ZERO);
    // 30 s at the visible cadence: 60 fast ticks, 30 slow, 15 process passes, 6 fan passes.
    for i in 0..60u64 {
        let at = Ts(i * 500_000_000);
        // The source's own schedule: the *next* multiple of each grid.
        let plan = Plan::for_tick(
            at,
            Level::Visible,
            Detail::Table,
            next_slow,
            next_fans,
            next_procs,
        );
        if plan.slow {
            next_slow = Ts((at.0 / 1_000_000_000 + 1) * 1_000_000_000);
        }
        if plan.fans {
            next_fans = Ts((at.0 / 5_000_000_000 + 1) * 5_000_000_000);
        }
        if plan.procs {
            next_procs = Ts((at.0 / 2_000_000_000 + 1) * 2_000_000_000);
        }
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        let out = p
            .tick(&mut probe, at, plan, st.num_fans, wall, TICK)
            .unwrap();
        // The per-class accounting is published on the 2 s grid.
        if scalar(&out, "gpu.nvml_ms{fast}").is_some() {
            last = Some(out);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let out = last.unwrap();
    let fast = scalar(&out, "gpu.nvml_ms{fast}").unwrap();
    let slow = scalar(&out, "gpu.nvml_ms{slow}").unwrap();
    let procs_ms = scalar(&out, "gpu.nvml_ms{procs}").unwrap();
    eprintln!(
        "P11: fast {fast:.2} + slow {slow:.2} + procs {procs_ms:.2} = {:.2} ms/s over {:?}",
        fast + slow + procs_ms,
        start.elapsed()
    );
    if let Some(t) = procs(&out) {
        eprintln!("rows: {:#?}", t.rows);
        assert!(
            t.rows.iter().all(|r| r.pid as u32 != std::process::id()),
            "P12"
        );
    }
    assert!(
        fast + slow + procs_ms <= 6.0,
        "P11: {} ms/s",
        fast + slow + procs_ms
    );
}

/// Live on torch (ignored in CI): the cost of every probe call, to audit P11's
/// sum against the digest's numbers. Run:
/// `cargo test -p gridwatch-sources --release --test gpu live_call_costs -- --ignored --nocapture`
#[test]
#[ignore = "needs the NVIDIA driver; run by hand on torch"]
fn live_call_costs() {
    use gridwatch_sources::gpu::nvml::{NvmlProbe, init};
    let nvml = init().expect("NVML");
    let mut p = NvmlProbe::open(&nvml, 0).expect("device 0");
    let wall = || {
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64
    };
    let time = |name: &str, mut f: Box<dyn FnMut() -> Result<String, Fail> + '_>| {
        let mut total = Duration::ZERO;
        let mut last = String::new();
        let n = 5;
        for _ in 0..n {
            let t0 = std::time::Instant::now();
            let r = f();
            total += t0.elapsed();
            last = match r {
                Ok(s) => s,
                Err(e) => format!("ERR {e}"),
            };
        }
        eprintln!(
            "{name:>16}: {:>8.3} ms  {last}",
            total.as_secs_f64() * 1000.0 / n as f64
        );
    };
    time(
        "utilization",
        Box::new(|| p.utilization().map(|v| format!("{v:?}"))),
    );
    time(
        "temperature",
        Box::new(|| p.temperature_c().map(|v| v.to_string())),
    );
    time("power_w", Box::new(|| p.power_w().map(|v| v.to_string())));
    time(
        "power_limit",
        Box::new(|| p.power_limit_w().map(|v| v.to_string())),
    );
    time(
        "clock_gfx",
        Box::new(|| p.clock_gfx_mhz().map(|v| v.to_string())),
    );
    time(
        "clock_mem",
        Box::new(|| p.clock_mem_mhz().map(|v| v.to_string())),
    );
    time("pstate", Box::new(|| p.pstate().map(|v| v.to_string())));
    time(
        "throttle",
        Box::new(|| p.throttle_bits().map(|v| format!("{v:#x}"))),
    );
    time(
        "memory",
        Box::new(|| p.memory_b().map(|v| format!("{v:?}"))),
    );
    time(
        "encoder",
        Box::new(|| p.encoder_pct().map(|v| v.to_string())),
    );
    time(
        "decoder",
        Box::new(|| p.decoder_pct().map(|v| v.to_string())),
    );
    time(
        "pcie_link",
        Box::new(|| p.pcie_link().map(|v| format!("{v:?}"))),
    );
    time(
        "pcie_bytes",
        Box::new(|| p.pcie_bytes().map(|v| format!("{v:?}"))),
    );
    time(
        "fan_pct(0)",
        Box::new(|| p.fan_pct(0).map(|v| v.to_string())),
    );
    time(
        "fan_rpm(0)",
        Box::new(|| p.fan_rpm(0).map(|v| v.to_string())),
    );
    time(
        "power_samples",
        Box::new(|| p.power_samples(0).map(|v| v.len().to_string())),
    );
    time(
        "graphics_procs",
        Box::new(|| p.graphics_procs().map(|v| v.len().to_string())),
    );
    time(
        "compute_procs",
        Box::new(|| p.compute_procs().map(|v| v.len().to_string())),
    );
    let w = wall() - 1_000_000;
    time(
        "proc_util",
        Box::new(|| p.proc_util(w).map(|v| v.len().to_string())),
    );
}
