//! Deterministic cpu/sys synthesis: 32 cores in two asymmetric CCDs with a
//! game-like load. Same seed → byte-identical batches.

use std::sync::Arc;
use std::time::Duration;

use crate::capability::Capability;
use crate::key::{Datum, Label, MetricId};
use crate::keys::{cpu, sys};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Level, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

/// xorshift64* — tiny, seedable, good enough for plausible wiggle.
#[derive(Clone, Debug)]
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> XorShift {
        // splitmix64 scramble so nearby seeds diverge (42 | 1 == 43 | 1 taught us).
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        XorShift((z ^ (z >> 31)) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [-1, 1).
    pub fn jitter(&mut self) -> f64 {
        self.f64() * 2.0 - 1.0
    }
}

const CORES: usize = 32;

/// Pure generator: `tick(at)` yields the cpu source's batch at that instant.
#[derive(Clone, Debug)]
pub struct CpuSynth {
    rng: XorShift,
    core: [f64; CORES],
    mem_used_frac: f64,
    /// The die map is published once per source generation, exactly as the live
    /// sampler does it — demo and live must emit the same keys (§12.5).
    topology_sent: bool,
}

impl CpuSynth {
    pub fn new(seed: u64) -> CpuSynth {
        CpuSynth {
            rng: XorShift::new(seed),
            core: [0.3; CORES],
            mem_used_frac: 0.18,
            topology_sent: false,
        }
    }

    fn busy_ccd(core: usize) -> bool {
        // CCD0 = cpus 0–7 and SMT siblings 16–23 (torch topology): the game side.
        (core % 16) < 8
    }

    /// Torch's die/core map (§8, D43): 16 physical cores, SMT sibling of cpu N
    /// is cpu N+16, die 0 = cpus 0–7 + 16–23.
    pub fn topology() -> cpu::CpuTopology {
        cpu::CpuTopology {
            die_of: (0..CORES as u16).map(|c| u16::from(c % 16 >= 8)).collect(),
            core_of: (0..CORES as u16).map(|c| c % 16).collect(),
            die_temp: vec!["k10temp:Tccd1".into(), "k10temp:Tccd2".into()],
        }
    }

    pub fn tick(&mut self, at: Ts) -> Batch {
        let mut samples = Vec::with_capacity(CORES * 3 + 24);
        let mut total = 0.0;
        for i in 0..CORES {
            let target = if Self::busy_ccd(i) { 0.72 } else { 0.16 };
            let load = &mut self.core[i];
            *load = (*load + (target - *load) * 0.2 + self.rng.jitter() * 0.08).clamp(0.01, 1.0);
            total += *load;
            samples.push(Sample {
                id: cpu::CORE_PCT.idx(i as u16).id,
                datum: Datum::Scalar(*load * 100.0),
            });
            let mhz = if Self::busy_ccd(i) { 4900.0 } else { 5400.0 } + self.rng.jitter() * 120.0;
            samples.push(Sample {
                id: cpu::FREQ_MHZ.idx(i as u16).id,
                datum: Datum::Scalar(mhz),
            });
            // Segments sum to the core's load exactly (htop draws nice/user/
            // kernel/virt and counts iowait as idle), so the bar and `core_pct`
            // can never disagree.
            let l = *load;
            samples.push(Sample {
                id: cpu::BREAKDOWN.idx(i as u16).id,
                datum: Datum::Record(Arc::new(cpu::CoreBreakdown {
                    nice: (l * 0.02) as f32,
                    user: (l * 0.75) as f32,
                    kernel: (l * 0.23) as f32,
                    virt: 0.0,
                    iowait: (l * 0.04) as f32,
                })),
            });
        }
        let total_pct = total / CORES as f64 * 100.0;
        samples.push(Sample {
            id: cpu::TOTAL_PCT.id.clone(),
            datum: Datum::Scalar(total_pct),
        });
        let agg = (total / CORES as f64) as f32;
        samples.push(Sample {
            id: cpu::BREAKDOWN.id.clone(),
            datum: Datum::Record(Arc::new(cpu::CoreBreakdown {
                nice: agg * 0.02,
                user: agg * 0.75,
                kernel: agg * 0.23,
                virt: 0.0,
                iowait: agg * 0.04,
            })),
        });

        let mem_total = 91.0 * 1024.0 * 1024.0 * 1024.0;
        self.mem_used_frac = (self.mem_used_frac + self.rng.jitter() * 0.002).clamp(0.15, 0.6);
        let used = mem_total * self.mem_used_frac;
        for (key, v) in [
            (&cpu::MEM_TOTAL_B, mem_total),
            (&cpu::MEM_USED_B, used),
            (&cpu::MEM_AVAILABLE_B, mem_total - used - mem_total * 0.05),
            (&cpu::MEM_CACHED_B, mem_total * 0.22),
            (&cpu::MEM_BUFFERS_B, mem_total * 0.01),
            (&cpu::MEM_SHARED_B, mem_total * 0.011),
            (&cpu::SWAP_TOTAL_B, 16.0 * 1024.0 * 1024.0 * 1024.0),
            (&cpu::SWAP_USED_B, 0.0),
            (&cpu::SWAP_CACHED_B, 0.0),
        ] {
            samples.push(Sample {
                id: key.id.clone(),
                datum: Datum::Scalar(v),
            });
        }

        let load1 = total / CORES as f64 * 8.0 + 1.0;
        for (key, v) in [
            (&sys::LOAD1, load1),
            (&sys::LOAD5, load1 * 0.93),
            (&sys::LOAD15, load1 * 0.78),
            (&sys::UPTIME_S, 3600.0 * 78.0 + at.as_secs_f64()),
            (&sys::PID_DIGITS, 7.0),
            (&sys::TASKS_TOTAL, 636.0 + (self.rng.f64() * 6.0).floor()),
            (
                &sys::TASKS_THREADS,
                2412.0 + (self.rng.f64() * 30.0).floor(),
            ),
            (&sys::TASKS_RUNNING, 2.0 + (self.rng.f64() * 4.0).floor()),
            // No `tasks.kernel`: counting kernel threads needs the pid-level
            // scan (PF_KTHREAD per pid), which is arc 2 — the live source
            // cannot emit it in 1b, so the synth must not either (§12.5).
            (
                &cpu::PSI_CPU,
                (total_pct / 40.0 + self.rng.f64()).clamp(0.0, 30.0),
            ),
            (&cpu::PSI_MEM, self.rng.f64() * 0.3),
            (&cpu::PSI_IO, self.rng.f64() * 1.2),
        ] {
            samples.push(Sample {
                id: key.id.clone(),
                datum: Datum::Scalar(v),
            });
        }

        for (label, base) in [
            ("k10temp:Tctl", 63.0),
            ("k10temp:Tccd1", 61.0),
            ("k10temp:Tccd2", 54.0),
        ] {
            let name: Arc<str> = Arc::from(label);
            samples.push(Sample {
                id: MetricId {
                    name: cpu::TEMP_C.id.name,
                    label: Label::Name(name),
                },
                datum: Datum::Scalar(base + self.rng.jitter() * 1.5),
            });
        }

        if !self.topology_sent {
            self.topology_sent = true;
            samples.push(Sample {
                id: cpu::TOPOLOGY.id.clone(),
                datum: Datum::Record(Arc::new(Self::topology())),
            });
        }

        Batch {
            source: cpu::SOURCE,
            at,
            samples,
        }
    }
}

pub fn cpu_info() -> SourceInfo {
    SourceInfo {
        id: cpu::SOURCE,
        produces: &[
            "cpu.*",
            "mem.*",
            "swap.*",
            "psi.*",
            "sys.*",
            "tasks.*",
            "sensor.temp_c{k10temp:*}",
        ],
        cadence: Cadence {
            hidden: Some(Duration::from_secs(3)),
            visible: Duration::from_millis(1500),
            focused: Duration::from_millis(500),
            always_on: false,
        },
        requires: &[Capability::Procfs],
    }
}

struct CpuDemoSource {
    seed: u64,
}

impl Source for CpuDemoSource {
    fn info(&self) -> SourceInfo {
        cpu_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = CpuSynth::new(self.seed);
        let info = self.info();
        // Prime the pump like the live source does (P18: every source live
        // within 2 s), so `--demo` paints data on the first frames — but a
        // paused source emits nothing, restart or not (§4.3).
        if cx.demand.level() != Level::Paused {
            let at = cx.clock.now();
            let first = synth.tick(at);
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
            let deadline = cx.next_deadline(cadence);
            if !cx.sleep_until(deadline) {
                return;
            }
            if level != Level::Paused {
                let at = cx.clock.now();
                let batch = synth.tick(at);
                cx.emit(at, batch.samples);
            }
        }
    }
}

/// The seeded demo source for `--demo` and `SourceDef.demo` (§4.3).
pub fn cpu_demo(seed: u64) -> Box<dyn Source> {
    Box::new(CpuDemoSource { seed })
}
