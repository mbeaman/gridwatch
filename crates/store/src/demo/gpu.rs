//! Deterministic gpu synthesis (§12.5, brief 2b task 3): a 5090 at ≈ 400 W of
//! a 600 W limit, 17 % SM with a game-like ten-minute swell so the charts have
//! shape as they fill, 13.9 of 32.6 GiB VRAM, three fans, P0, and the process
//! set from 2a with matching VRAM/SM. Same seed → byte-identical batches. It
//! publishes exactly the keys the live source publishes at each detail.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use crate::capability::Capability;
use crate::demo::XorShift;
use crate::key::{Datum, Label, MetricId};
use crate::keys::gpu::{self, GpuInfo, GpuProcKind, GpuProcRow, GpuProcs, GpuSpec, Throttle};
use crate::msg::{Batch, Sample};
use crate::source::{
    Cadence, Detail, Level, Source, SourceCtx, SourceInfo, SourceState, SourceStatus,
};
use crate::ts::Ts;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
pub const VRAM_TOTAL_B: u64 = 32_607 * 1024 * 1024;
const POWER_LIMIT_W: f64 = 600.0;
const FANS: u16 = 3;

/// The synthetic 5090's static row — the same numbers the live `SPECS` table
/// carries for `0x2B85`, so a demo screenshot and torch agree.
pub fn info() -> GpuInfo {
    GpuInfo {
        name: "NVIDIA GeForce RTX 5090".into(),
        driver: "610.57.04".into(),
        cuda: 13030,
        arch: "Blackwell".into(),
        uuid: "GPU-0f0f0f0f-demo-4242-8888-5090a5090a50".into(),
        pci_id: 0x2B85,
        bus_id: "00000000:01:00.0".into(),
        vbios: "98.02.2E.80.05".into(),
        cores: Some(21760),
        bus_width: Some(512),
        spec: Some(GpuSpec {
            sms: 170,
            tmus: 680,
            rops: 176,
            rt_cores: 170,
            tensor_cores: 680,
            l2_mb: 96,
            base_mhz: 2017,
            boost_mhz: 2407,
            mem_gbps: 28.0,
            bandwidth_gbs: 1792,
            tdp_w: 575,
            die_mm2: 750,
            transistors_b: 92.2,
            launch: Cow::Borrowed("2025-01-30"),
        }),
        spec_mismatch: false,
    }
}

/// The GPU side of the 2a process set (§8.1): the game holds 12.5 GiB and
/// ≈ 17 % SM in both lists, the terminal a compute context at 44 MiB with no
/// fresh sample, the desktop and browser graphics contexts. PIDs match
/// `demo::proc_table` so the join has rows to land on.
pub fn gpu_procs(tick: u64, seed: u64) -> GpuProcs {
    let mut rng = XorShift::new(seed ^ tick.wrapping_mul(0x517C_C1B7_2722_0A95));
    let sm = |base: u32, spread: u32, rng: &mut XorShift| -> u32 {
        (base as f64 + rng.jitter() * spread as f64)
            .round()
            .max(0.0) as u32
    };
    let mib = |m: u64| Some(m * 1024 * 1024);
    let game_sm = sm(17, 2, &mut rng);
    let shell_sm = sm(3, 1, &mut rng);
    let rows = vec![
        GpuProcRow {
            pid: 412345,
            kind: GpuProcKind::Both,
            vram_b: Some((12.5 * GIB) as u64 + 79 * 1024 * 1024),
            sm_pct: game_sm,
            mem_pct: sm(9, 1, &mut rng),
            enc_pct: 0,
            dec_pct: 0,
            fresh: true,
        },
        GpuProcRow {
            pid: 1701,
            kind: GpuProcKind::Graphics,
            vram_b: mib(464),
            sm_pct: shell_sm,
            mem_pct: 1,
            enc_pct: 0,
            dec_pct: 0,
            fresh: true,
        },
        GpuProcRow {
            pid: 6120,
            kind: GpuProcKind::Graphics,
            vram_b: mib(318),
            sm_pct: 0,
            mem_pct: 0,
            enc_pct: 0,
            dec_pct: 0,
            fresh: false,
        },
        GpuProcRow {
            pid: 11805,
            kind: GpuProcKind::Compute,
            vram_b: mib(44),
            sm_pct: 0,
            mem_pct: 0,
            enc_pct: 0,
            dec_pct: 0,
            fresh: false,
        },
        GpuProcRow {
            pid: 1842,
            kind: GpuProcKind::Graphics,
            vram_b: mib(12),
            sm_pct: 0,
            mem_pct: 0,
            enc_pct: 0,
            dec_pct: 0,
            fresh: false,
        },
    ];
    GpuProcs {
        rows,
        vram_total_b: VRAM_TOTAL_B,
    }
}

/// Pure generator: `tick_at(at, detail)` yields the gpu source's batch.
#[derive(Clone, Debug)]
pub struct GpuSynth {
    rng: XorShift,
    seed: u64,
    ticks: u64,
    info_sent: bool,
    /// Slow-tier keys go out every second tick like the live source's 1 s
    /// grid under a 500 ms fast tier.
    fans_sent: bool,
    table_sent: bool,
    vram_frac: f64,
    /// The monotonic PCIe counters the live source diffs; kept so the synth's
    /// rates have the same texture.
    pcie_rx: f64,
    pcie_tx: f64,
}

impl GpuSynth {
    /// The static row the synth publishes (tests' journal exemplar).
    pub fn info_exemplar() -> GpuInfo {
        info()
    }

    pub fn new(seed: u64) -> GpuSynth {
        GpuSynth {
            rng: XorShift::new(seed.wrapping_add(0x0067_7075)),
            seed,
            ticks: 0,
            info_sent: false,
            fans_sent: false,
            table_sent: false,
            vram_frac: 13.9 / 32.6,
            pcie_rx: 0.0,
            pcie_tx: 0.0,
        }
    }

    /// The game's load curve: a ten-minute swell between ≈ 12 and ≈ 24 % SM
    /// with a faster ripple, phase-shifted so the first minute is already on a
    /// slope. Util is the only number the charts *need* to move.
    fn util_at(&mut self, at: Ts) -> f64 {
        let t = at.as_secs_f64();
        let slow = ((t / 600.0 + 0.6) * std::f64::consts::TAU).sin();
        let fast = ((t / 47.0) * std::f64::consts::TAU).sin();
        (17.0 + slow * 5.0 + fast * 1.5 + self.rng.jitter() * 0.8).clamp(0.0, 100.0)
    }

    pub fn tick_at(&mut self, at: Ts, detail: Detail) -> Batch {
        let tick = self.ticks;
        self.ticks += 1;
        let dev = 0u16;
        let mut samples: Vec<Sample> = Vec::with_capacity(40);
        fn scalar(samples: &mut Vec<Sample>, dev: u16, key: &crate::key::Key<f64>, v: f64) {
            samples.push(Sample {
                id: key.idx(dev).id,
                datum: Datum::Scalar(v),
            });
        }
        let util = self.util_at(at);
        let power = 300.0 + util * 6.0 + self.rng.jitter() * 6.0;
        let temp = 41.0 + util * 0.25 + self.rng.jitter() * 0.4;
        scalar(&mut samples, dev, &gpu::UTIL_PCT, util);
        scalar(&mut samples, dev, &gpu::TEMP_C, temp);
        scalar(&mut samples, dev, &gpu::POWER_W, power);
        scalar(&mut samples, dev, &gpu::POWER_LIMIT_W, POWER_LIMIT_W);
        scalar(
            &mut samples,
            dev,
            &gpu::CLOCK_GFX_MHZ,
            2790.0 + self.rng.jitter() * 30.0,
        );
        scalar(&mut samples, dev, &gpu::CLOCK_MEM_MHZ, 14001.0);
        scalar(&mut samples, dev, &gpu::PSTATE, 0.0);
        samples.push(Sample {
            id: gpu::THROTTLE.idx(dev).id,
            datum: Datum::Record(Arc::new(Throttle {
                bits: if power > 590.0 {
                    Throttle::SW_POWER_CAP
                } else {
                    0
                },
            })),
        });

        // Slow tier: every second tick, and always the first so a tile is not
        // left waiting a tick for its VRAM number.
        if tick.is_multiple_of(2) {
            self.vram_frac = (self.vram_frac + self.rng.jitter() * 0.0015).clamp(0.40, 0.46);
            scalar(
                &mut samples,
                dev,
                &gpu::MEMCTL_PCT,
                (util * 0.3 + self.rng.jitter()).max(0.0),
            );
            scalar(
                &mut samples,
                dev,
                &gpu::VRAM_USED_B,
                (VRAM_TOTAL_B as f64 * self.vram_frac).round(),
            );
            scalar(&mut samples, dev, &gpu::VRAM_TOTAL_B, VRAM_TOTAL_B as f64);
            scalar(&mut samples, dev, &gpu::ENC_PCT, 0.0);
            scalar(&mut samples, dev, &gpu::DEC_PCT, 0.0);
            scalar(&mut samples, dev, &gpu::PCIE_GEN, 5.0);
            scalar(&mut samples, dev, &gpu::PCIE_WIDTH, 16.0);
            let rx = (12.0 + util * 1.5 + self.rng.f64() * 4.0) * 1024.0 * 1024.0;
            let tx = (3.0 + util * 0.4 + self.rng.f64() * 2.0) * 1024.0 * 1024.0;
            self.pcie_rx += rx;
            self.pcie_tx += tx;
            scalar(&mut samples, dev, &gpu::PCIE_RX_BPS, rx);
            scalar(&mut samples, dev, &gpu::PCIE_TX_BPS, tx);
            // 50 samples at 20 ms around the current power.
            let trace: Vec<f32> = (0..50)
                .map(|i| {
                    let ripple = ((i as f64 / 50.0) * std::f64::consts::TAU * 3.0).sin() * 8.0;
                    (power + ripple + self.rng.jitter() * 3.0) as f32
                })
                .collect();
            samples.push(Sample {
                id: gpu::POWER_TRACE.idx(dev).id,
                datum: Datum::Vector(Arc::from(trace)),
            });
            for (class, ms) in [("fast", 0.04), ("slow", 2.3), ("procs", 0.0)] {
                let v = if class == "procs" && detail >= Detail::Table {
                    2.0
                } else {
                    ms
                };
                samples.push(Sample {
                    id: MetricId {
                        name: gpu::NVML_MS.id.name,
                        label: Label::Name(Arc::from(class)),
                    },
                    datum: Datum::Scalar(v + self.rng.f64() * 0.1),
                });
            }
        }
        // Fans every 5 s → every tenth tick at 500 ms; always the first.
        if tick.is_multiple_of(10) || !self.fans_sent {
            self.fans_sent = true;
            for fan in 0..FANS {
                let pct = 30.0 + f64::from(fan % 2) + (util - 17.0) * 0.4;
                for (key, v) in [(&gpu::FAN_PCT, pct), (&gpu::FAN_RPM, pct * 17.2)] {
                    samples.push(Sample {
                        id: MetricId {
                            name: key.id.name,
                            label: Label::Name(gpu::fan_label(dev, fan)),
                        },
                        datum: Datum::Scalar(v),
                    });
                }
            }
        }
        if !self.info_sent {
            self.info_sent = true;
            scalar(&mut samples, dev, &gpu::TEMP_SLOWDOWN_C, 93.0);
            scalar(&mut samples, dev, &gpu::CLOCK_GFX_MAX_MHZ, 3135.0);
            scalar(&mut samples, dev, &gpu::CLOCK_MEM_MAX_MHZ, 14001.0);
            samples.push(Sample {
                id: gpu::INFO.idx(dev).id,
                datum: Datum::Record(Arc::new(info())),
            });
        }
        if detail >= Detail::Table && (tick.is_multiple_of(2) || !self.table_sent) {
            self.table_sent = true;
            samples.push(Sample {
                id: gpu::PROCS.idx(dev).id,
                datum: Datum::Record(Arc::new(gpu_procs(tick, self.seed))),
            });
        }
        Batch {
            source: gpu::SOURCE,
            at,
            samples,
        }
    }
}

pub fn gpu_info() -> SourceInfo {
    SourceInfo {
        id: gpu::SOURCE,
        produces: &["gpu.*"],
        cadence: Cadence {
            hidden: Some(Duration::from_secs(1)),
            visible: Duration::from_millis(500),
            focused: Duration::from_millis(250),
            always_on: false,
        },
        requires: &[Capability::Nvml],
    }
}

struct GpuDemoSource {
    seed: u64,
}

impl Source for GpuDemoSource {
    fn info(&self) -> SourceInfo {
        gpu_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = GpuSynth::new(self.seed);
        let info = self.info();
        if cx.demand.level() != Level::Paused {
            let at = cx.clock.now();
            let first = synth.tick_at(at, cx.demand.detail());
            cx.emit(at, first.samples);
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
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
            let level = cx.demand.level();
            let cadence = info
                .cadence
                .for_level(level)
                .unwrap_or(Duration::from_secs(1));
            if !cx.sleep_until(cx.next_deadline(cadence)) {
                return;
            }
            if level != Level::Paused {
                let at = cx.clock.now();
                let batch = synth.tick_at(at, cx.demand.detail());
                cx.emit(at, batch.samples);
            }
        }
    }
}

/// The seeded demo source for `--demo` and `SourceDef.demo` (§4.3).
pub fn gpu_demo(seed: u64) -> Box<dyn Source> {
    Box::new(GpuDemoSource { seed })
}
