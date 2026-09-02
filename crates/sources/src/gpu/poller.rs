//! The tier logic (§5 cadence row, §8, P11–P13) over any `Probe`: the fast
//! tier every tick, the slow tier on its 1 s grid, fans every 5 s, the power
//! trace while a gpu tile is visible, process rows only at `Detail::Table`;
//! per-field `NotSupported` pruning, PCIe byte counters diffed to B/s, the
//! utilisation `last_seen` carried forward, and wall time per call class for
//! the `sources` tile.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gridwatch_store::keys::gpu::{self, GpuInfo, PSTATE_UNKNOWN, Throttle};
use gridwatch_store::{Datum, Detail, Label, Level, MetricId, Sample, Ts};

use super::probe::{Fail, Probe, Static};
use super::procs;
use super::specs;

/// Every pollable field, for the pruning bitset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Field {
    Utilization,
    Temperature,
    Power,
    PowerLimit,
    ClockGfx,
    ClockMem,
    Pstate,
    Throttle,
    Memory,
    Encoder,
    Decoder,
    PcieLink,
    PcieBytes,
    PowerSamples,
    GraphicsProcs,
    ComputeProcs,
    ProcUtil,
    /// Fan `i` is bit `FAN0 + i`; fans beyond `MAX_FANS` are never polled.
    Fan0,
}

pub const MAX_FANS: u32 = 8;

/// Which call class a field's wall time is charged to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Fast,
    Slow,
    Procs,
}

impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Class::Fast => "fast",
            Class::Slow => "slow",
            Class::Procs => "procs",
        }
    }
}

/// What one tick should include beyond the fast tier.
#[derive(Clone, Copy, Debug, Default)]
pub struct Plan {
    pub slow: bool,
    pub fans: bool,
    /// `samples(Power)`: while a gpu tile is visible (D49).
    pub power_trace: bool,
    /// The v3 lists and `process_utilization_stats`: `Detail::Table`+ only
    /// (P13), on their own 2 s grid (D49: 2.2 ms per call, and the joined
    /// table only changes when the 3 s cpu scan does).
    pub procs: bool,
}

impl Plan {
    pub fn for_tick(
        at: Ts,
        level: Level,
        detail: Detail,
        next_slow: Ts,
        next_fans: Ts,
        next_procs: Ts,
    ) -> Plan {
        let slow = at >= next_slow;
        Plan {
            slow,
            fans: slow && at >= next_fans,
            power_trace: slow && level >= Level::Visible,
            procs: slow && detail >= Detail::Table && at >= next_procs,
        }
    }
}

/// The slow tier's grid period.
pub const SLOW_PERIOD: Duration = Duration::from_secs(1);
/// Fans %/RPM cadence.
pub const FAN_PERIOD: Duration = Duration::from_secs(5);
/// Process rows cadence (D49).
pub const PROCS_PERIOD: Duration = Duration::from_secs(2);

/// Timing per class since the last slow tick.
#[derive(Clone, Copy, Debug, Default)]
struct Spent {
    fast: Duration,
    slow: Duration,
    procs: Duration,
    /// Window start on the store's clock (deterministic in tests).
    since: Option<Ts>,
}

/// A monotonic counter's step, allowing for a 32-bit wrap: the PCIe byte
/// counters on driver 610 sit just under 2^32 and roll over every few
/// seconds on this desktop (review: rates read `0 B/s` at every wrap).
pub fn counter_delta(prev: u64, now: u64) -> u64 {
    if now >= prev {
        now - prev
    } else if prev < (1u64 << 32) {
        (now + (1u64 << 32)) - prev
    } else {
        0
    }
}

pub struct Poller {
    dev: u16,
    own_pid: u32,
    pruned: u64,
    prev_pcie: Option<(Ts, u64, u64)>,
    last_power_ts: u64,
    /// `process_utilization_stats`' `last_seen`, microseconds of CPU wall
    /// clock; `None` until the first process tick sets `wall_us − tick_us`.
    last_seen_us: Option<u64>,
    /// Kept across an `InsufficientSize` so the table does not blink.
    prev_rows: Vec<gpu::GpuProcRow>,
    vram_total_b: u64,
    spent: Spent,
    warned_other: bool,
    last_fast: Duration,
}

impl Poller {
    /// Wall time the last tick's fast tier cost — the source drops the focused
    /// cadence back to the visible one while this is over a millisecond (an
    /// idle card in P8 answers `utilization_rates` in 0.65 ms, not 1 µs; D49).
    pub fn last_fast(&self) -> Duration {
        self.last_fast
    }

    pub fn new(dev: u16, own_pid: u32) -> Poller {
        Poller {
            dev,
            own_pid,
            pruned: 0,
            prev_pcie: None,
            last_power_ts: 0,
            last_seen_us: None,
            prev_rows: Vec::new(),
            vram_total_b: 0,
            spent: Spent::default(),
            warned_other: false,
            last_fast: Duration::ZERO,
        }
    }

    fn bit(f: Field, fan: u32) -> u64 {
        1u64 << (f as u8 as u32 + fan)
    }

    pub fn is_pruned(&self, f: Field) -> bool {
        self.pruned & Self::bit(f, 0) != 0
    }

    fn charge(&mut self, class: Class, d: Duration) {
        match class {
            Class::Fast => self.spent.fast += d,
            Class::Slow => self.spent.slow += d,
            Class::Procs => self.spent.procs += d,
        }
    }

    /// One probe call with the pruning contract: `NotSupported` prunes the
    /// field for good; `NotFound`/`InsufficientSize`/`Other` are transient
    /// (`None` this tick); `GpuLost`/`Mismatch`/`Loading` abort the tick.
    fn call<T>(
        &mut self,
        field: Field,
        fan: u32,
        class: Class,
        f: impl FnOnce() -> Result<T, Fail>,
    ) -> Result<Option<T>, Fail> {
        if self.pruned & Self::bit(field, fan) != 0 {
            return Ok(None);
        }
        let t0 = Instant::now();
        let r = f();
        self.charge(class, t0.elapsed());
        match r {
            Ok(v) => Ok(Some(v)),
            Err(Fail::NotSupported) => {
                self.pruned |= Self::bit(field, fan);
                tracing::info!(?field, fan, "not supported — pruned");
                Ok(None)
            }
            Err(Fail::NotFound) | Err(Fail::InsufficientSize) => Ok(None),
            Err(Fail::Other(s)) => {
                if !self.warned_other {
                    self.warned_other = true;
                    tracing::warn!(?field, "nvml: {s}");
                }
                Ok(None)
            }
            Err(fatal) => Err(fatal),
        }
    }

    fn id(&self, key: &gridwatch_store::Key<f64>) -> MetricId {
        key.idx(self.dev).id
    }

    fn scalar(&self, out: &mut Vec<Sample>, key: &gridwatch_store::Key<f64>, v: f64) {
        out.push(Sample {
            id: self.id(key),
            datum: Datum::Scalar(v),
        });
    }

    /// The static samples for a generation: `gpu.info` with the spec
    /// cross-check, the max clocks and the slowdown threshold.
    pub fn static_samples(&self, st: &Static) -> Vec<Sample> {
        let (spec, spec_mismatch) = specs::cross_check(st.pci_id, st.cores, st.bus_width);
        let info = GpuInfo {
            name: st.name.clone(),
            driver: st.driver.clone(),
            cuda: st.cuda,
            arch: st.arch.clone(),
            uuid: st.uuid.clone(),
            pci_id: st.pci_id,
            bus_id: st.bus_id.clone(),
            vbios: st.vbios.clone(),
            cores: st.cores,
            bus_width: st.bus_width,
            spec,
            spec_mismatch,
        };
        let mut out = vec![Sample {
            id: gpu::INFO.idx(self.dev).id,
            datum: Datum::Record(Arc::new(info)),
        }];
        if let Some(v) = st.clock_gfx_max_mhz {
            self.scalar(&mut out, &gpu::CLOCK_GFX_MAX_MHZ, f64::from(v));
        }
        if let Some(v) = st.clock_mem_max_mhz {
            self.scalar(&mut out, &gpu::CLOCK_MEM_MAX_MHZ, f64::from(v));
        }
        if let Some(v) = st.temp_slowdown_c {
            self.scalar(&mut out, &gpu::TEMP_SLOWDOWN_C, f64::from(v));
        }
        out
    }

    /// One tick. `wall_us` is the CPU wall clock in microseconds (the
    /// `last_seen` currency); `tick` the fast period, for the first
    /// `last_seen = wall_us − tick_us`.
    pub fn tick(
        &mut self,
        probe: &mut dyn Probe,
        at: Ts,
        plan: Plan,
        num_fans: u32,
        wall_us: u64,
        tick: Duration,
    ) -> Result<Vec<Sample>, Fail> {
        let mut out = Vec::with_capacity(32);
        if self.spent.since.is_none() {
            self.spent.since = Some(at);
        }
        // Fast tier.
        let fast_t0 = Instant::now();
        if let Some((gpu_pct, mem_pct)) =
            self.call(Field::Utilization, 0, Class::Fast, || probe.utilization())?
        {
            self.scalar(&mut out, &gpu::UTIL_PCT, f64::from(gpu_pct));
            // Memory-controller utilisation rides the fast call; nvtop's MEM
            // bar is VRAM occupancy from the slow tier (digest §1).
            self.scalar(&mut out, &gpu::MEMCTL_PCT, f64::from(mem_pct));
        }
        if let Some(t) = self.call(Field::Temperature, 0, Class::Fast, || probe.temperature_c())? {
            self.scalar(&mut out, &gpu::TEMP_C, f64::from(t));
        }
        if let Some(w) = self.call(Field::Power, 0, Class::Fast, || probe.power_w())? {
            self.scalar(&mut out, &gpu::POWER_W, w);
        }
        if let Some(w) = self.call(Field::PowerLimit, 0, Class::Fast, || probe.power_limit_w())? {
            self.scalar(&mut out, &gpu::POWER_LIMIT_W, w);
        }
        if let Some(c) = self.call(Field::ClockGfx, 0, Class::Fast, || probe.clock_gfx_mhz())? {
            self.scalar(&mut out, &gpu::CLOCK_GFX_MHZ, f64::from(c));
        }
        if let Some(c) = self.call(Field::ClockMem, 0, Class::Fast, || probe.clock_mem_mhz())? {
            self.scalar(&mut out, &gpu::CLOCK_MEM_MHZ, f64::from(c));
        }
        if let Some(p) = self.call(Field::Pstate, 0, Class::Fast, || probe.pstate())? {
            let v = if p > 15 { PSTATE_UNKNOWN } else { f64::from(p) };
            self.scalar(&mut out, &gpu::PSTATE, v);
        }
        if let Some(bits) = self.call(Field::Throttle, 0, Class::Fast, || probe.throttle_bits())? {
            out.push(Sample {
                id: gpu::THROTTLE.idx(self.dev).id,
                datum: Datum::Record(Arc::new(Throttle { bits })),
            });
        }
        self.last_fast = fast_t0.elapsed();
        if !plan.slow {
            return Ok(out);
        }
        // Slow tier.
        if let Some((used, total)) =
            self.call(Field::Memory, 0, Class::Slow, || probe.memory_b())?
        {
            self.vram_total_b = total;
            self.scalar(&mut out, &gpu::VRAM_USED_B, used as f64);
            self.scalar(&mut out, &gpu::VRAM_TOTAL_B, total as f64);
        }
        if let Some(e) = self.call(Field::Encoder, 0, Class::Slow, || probe.encoder_pct())? {
            self.scalar(&mut out, &gpu::ENC_PCT, f64::from(e));
        }
        if let Some(d) = self.call(Field::Decoder, 0, Class::Slow, || probe.decoder_pct())? {
            self.scalar(&mut out, &gpu::DEC_PCT, f64::from(d));
        }
        if let Some((generation, width)) =
            self.call(Field::PcieLink, 0, Class::Slow, || probe.pcie_link())?
        {
            self.scalar(&mut out, &gpu::PCIE_GEN, f64::from(generation));
            self.scalar(&mut out, &gpu::PCIE_WIDTH, f64::from(width));
        }
        if let Some((tx, rx)) =
            self.call(Field::PcieBytes, 0, Class::Slow, || probe.pcie_bytes())?
        {
            if let Some((t0, tx0, rx0)) = self.prev_pcie {
                let secs = at.since(t0).as_secs_f64();
                if secs > 0.0 {
                    self.scalar(
                        &mut out,
                        &gpu::PCIE_TX_BPS,
                        counter_delta(tx0, tx) as f64 / secs,
                    );
                    self.scalar(
                        &mut out,
                        &gpu::PCIE_RX_BPS,
                        counter_delta(rx0, rx) as f64 / secs,
                    );
                }
            }
            self.prev_pcie = Some((at, tx, rx));
        }
        if plan.fans {
            for fan in 0..num_fans.min(MAX_FANS) {
                let label = Label::Name(gpu::fan_label(self.dev, fan as u16));
                if let Some(p) = self.call(Field::Fan0, fan, Class::Slow, || probe.fan_pct(fan))? {
                    out.push(Sample {
                        id: MetricId {
                            name: gpu::FAN_PCT.id.name,
                            label: label.clone(),
                        },
                        datum: Datum::Scalar(f64::from(p)),
                    });
                }
                // RPM shares the fan's prune bit shifted past the % range.
                if let Some(r) = self.call(Field::Fan0, MAX_FANS + fan, Class::Slow, || {
                    probe.fan_rpm(fan)
                })? {
                    out.push(Sample {
                        id: MetricId {
                            name: gpu::FAN_RPM.id.name,
                            label,
                        },
                        datum: Datum::Scalar(f64::from(r)),
                    });
                }
            }
        }
        if plan.power_trace {
            let last = self.last_power_ts;
            if let Some(samples) = self.call(Field::PowerSamples, 0, Class::Slow, || {
                probe.power_samples(last)
            })? && !samples.is_empty()
            {
                self.last_power_ts = samples.iter().map(|(t, _)| *t).max().unwrap_or(last);
                let trace: Vec<f32> = samples.iter().map(|(_, w)| *w).collect();
                out.push(Sample {
                    id: gpu::POWER_TRACE.idx(self.dev).id,
                    datum: Datum::Vector(Arc::from(trace)),
                });
            }
        }
        if plan.procs {
            self.procs(probe, wall_us, tick, &mut out)?;
        }
        // P11 evidence: ms per second per class, averaged over a window of
        // at least one process grid (2 s) so a class on a slower grid reads
        // its true per-second cost rather than alternating with zero; the
        // first tick's window was its own duration and read 999 ms/s (review).
        if let Some(since) = self.spent.since
            && at.since(since) >= PROCS_PERIOD
        {
            let secs = at.since(since).as_secs_f64();
            for (class, d) in [
                (Class::Fast, self.spent.fast),
                (Class::Slow, self.spent.slow),
                (Class::Procs, self.spent.procs),
            ] {
                out.push(Sample {
                    id: MetricId {
                        name: gpu::NVML_MS.id.name,
                        label: Label::Name(Arc::from(class.label())),
                    },
                    datum: Datum::Scalar(d.as_secs_f64() * 1000.0 / secs),
                });
            }
            self.spent = Spent {
                since: Some(at),
                ..Spent::default()
            };
        }
        Ok(out)
    }

    fn procs(
        &mut self,
        probe: &mut dyn Probe,
        wall_us: u64,
        tick: Duration,
        out: &mut Vec<Sample>,
    ) -> Result<(), Fail> {
        let g = self.call(Field::GraphicsProcs, 0, Class::Procs, || {
            probe.graphics_procs()
        })?;
        let c = self.call(Field::ComputeProcs, 0, Class::Procs, || {
            probe.compute_procs()
        })?;
        let both_pruned =
            self.is_pruned(Field::GraphicsProcs) && self.is_pruned(Field::ComputeProcs);
        if both_pruned {
            return Ok(());
        }
        // `InsufficientSize` on either list: keep the previous rows this tick.
        let mut rows = match (g, c) {
            (Some(g), Some(c)) => procs::merge(&g, &c, self.own_pid),
            (Some(g), None) if self.is_pruned(Field::ComputeProcs) => {
                procs::merge(&g, &[], self.own_pid)
            }
            (None, Some(c)) if self.is_pruned(Field::GraphicsProcs) => {
                procs::merge(&[], &c, self.own_pid)
            }
            _ => self.prev_rows.clone(),
        };
        let last_seen = self
            .last_seen_us
            .unwrap_or_else(|| wall_us.saturating_sub(tick.as_micros() as u64));
        if let Some(samples) = self.call(Field::ProcUtil, 0, Class::Procs, || {
            probe.proc_util(last_seen)
        })? {
            let newest = procs::overlay(&mut rows, &samples, last_seen);
            self.last_seen_us = Some(newest);
        } else if self.last_seen_us.is_none() {
            self.last_seen_us = Some(last_seen);
        }
        self.prev_rows = rows.clone();
        out.push(Sample {
            id: gpu::PROCS.idx(self.dev).id,
            datum: Datum::Record(Arc::new(procs::table(rows, self.vram_total_b))),
        });
        Ok(())
    }
}
