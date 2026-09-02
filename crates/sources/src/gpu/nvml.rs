//! The nvml-wrapper backend (digest §2–3, verified on driver 610): `Nvml` and
//! the `Device` it lends live on the source thread and never leave it. Every
//! call here is one probe method; the field ids the digest measured are the
//! ones used — `POWER_INSTANT` (186), the PCIe byte counters (197/198) — and
//! `pcie_throughput` (21 ms per direction) is never called.

use nvml_wrapper::Nvml;
use nvml_wrapper::device::Device;
use nvml_wrapper::enum_wrappers::device::{
    Clock, PerformanceState, Sampling, TemperatureSensor, TemperatureThreshold,
};
use nvml_wrapper::enums::device::{SampleValue, UsedGpuMemory};
use nvml_wrapper::error::NvmlError;
use nvml_wrapper::structs::device::FieldId;

use super::probe::{Fail, Probe, ProcMem, ProcUtil, Static};

/// `NVML_FI_DEV_POWER_INSTANT`: 3 µs, unlike `POWER_AVERAGE` (3.3 ms).
pub const FI_POWER_INSTANT: u32 = 186;
/// `NVML_FI_DEV_PCIE_COUNT_TX_BYTES` / `RX_BYTES`: monotonic counters in
/// 0.2–0.4 ms; `pcie_throughput` blocks 21 ms per direction.
pub const FI_PCIE_TX_BYTES: u32 = 197;
pub const FI_PCIE_RX_BYTES: u32 = 198;

pub fn map_err(e: NvmlError) -> Fail {
    match e {
        NvmlError::NotSupported => Fail::NotSupported,
        NvmlError::NotFound => Fail::NotFound,
        NvmlError::InsufficientSize(_) => Fail::InsufficientSize,
        NvmlError::GpuLost => Fail::GpuLost,
        NvmlError::LibRmVersionMismatch => Fail::Mismatch,
        NvmlError::LibloadingError(e) => Fail::Loading(e.to_string()),
        NvmlError::LibraryNotFound => Fail::Loading("library not found".into()),
        NvmlError::FailedToLoadSymbol(s) => Fail::Loading(s),
        // 26 = DEPRECATED on newer drivers: the field is gone, treat as absent.
        NvmlError::UnexpectedVariant(26) => Fail::NotSupported,
        other => Fail::Other(other.to_string()),
    }
}

/// One device handle for a generation: `device_by_index` costs ≈ 6 ms (digest
/// §3) and the first live pass paid it on every call — 29 ms/s against P11's
/// 6 — so the `Device` is fetched once and borrowed from the `Nvml` the source
/// thread keeps on its stack. Neither leaves that thread.
pub struct NvmlProbe<'a> {
    nvml: &'a Nvml,
    dev: Device<'a>,
    /// `POWER_INSTANT` said `NotSupported` once: `power_usage` from then on,
    /// never the field again (review: the fallback re-asked every tick).
    instant_unsupported: bool,
}

/// `Nvml::init()` on the caller's thread (≈ 4 ms).
pub fn init() -> Result<Nvml, Fail> {
    Nvml::init().map_err(map_err)
}

impl<'a> NvmlProbe<'a> {
    pub fn open(nvml: &'a Nvml, index: u32) -> Result<NvmlProbe<'a>, Fail> {
        let dev = nvml.device_by_index(index).map_err(map_err)?;
        Ok(NvmlProbe {
            nvml,
            dev,
            instant_unsupported: false,
        })
    }

    fn field_u64(&self, id: u32) -> Result<u64, Fail> {
        let values = self.dev.field_values_for(&[FieldId(id)]).map_err(map_err)?;
        let sample = values.into_iter().next().ok_or(Fail::NotSupported)?;
        let sample = sample.map_err(map_err)?;
        match sample.value.map_err(map_err)? {
            SampleValue::U64(v) => Ok(v),
            SampleValue::U32(v) => Ok(u64::from(v)),
            SampleValue::I64(v) => Ok(v.max(0) as u64),
            SampleValue::F64(v) => Ok(v.max(0.0) as u64),
        }
    }

    fn procs(
        list: Result<Vec<nvml_wrapper::struct_wrappers::device::ProcessInfo>, NvmlError>,
    ) -> Result<Vec<ProcMem>, Fail> {
        Ok(list
            .map_err(map_err)?
            .into_iter()
            .map(|p| ProcMem {
                pid: p.pid,
                vram_b: match p.used_gpu_memory {
                    UsedGpuMemory::Used(b) => Some(b),
                    UsedGpuMemory::Unavailable => None,
                },
            })
            .collect())
    }
}

fn pstate_num(p: PerformanceState) -> u8 {
    use PerformanceState::*;
    match p {
        Zero => 0,
        One => 1,
        Two => 2,
        Three => 3,
        Four => 4,
        Five => 5,
        Six => 6,
        Seven => 7,
        Eight => 8,
        Nine => 9,
        Ten => 10,
        Eleven => 11,
        Twelve => 12,
        Thirteen => 13,
        Fourteen => 14,
        Fifteen => 15,
        Unknown => 32,
    }
}

/// `Option` for the fields a card may not support: never fatal at start.
fn opt<T>(r: Result<T, NvmlError>) -> Result<Option<T>, Fail> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) => match map_err(e) {
            Fail::NotSupported | Fail::NotFound | Fail::Other(_) => Ok(None),
            fatal => Err(fatal),
        },
    }
}

impl Probe for NvmlProbe<'_> {
    fn kind(&self) -> &'static str {
        "nvml"
    }

    fn static_info(&mut self) -> Result<Static, Fail> {
        let dev = &self.dev;
        let pci = dev.pci_info().map_err(map_err)?;
        Ok(Static {
            name: opt(dev.name())?.unwrap_or_default(),
            driver: opt(self.nvml.sys_driver_version())?.unwrap_or_default(),
            cuda: opt(self.nvml.sys_cuda_driver_version())?
                .map(|v| v.max(0) as u32)
                .unwrap_or(0),
            arch: opt(dev.architecture())?
                .map(|a| format!("{a:?}"))
                .unwrap_or_default(),
            uuid: opt(dev.uuid())?.unwrap_or_default(),
            pci_id: pci.pci_device_id >> 16,
            bus_id: pci.bus_id,
            vbios: opt(dev.vbios_version())?.unwrap_or_default(),
            cores: opt(dev.num_cores())?,
            bus_width: opt(dev.memory_bus_width())?,
            clock_gfx_max_mhz: opt(dev.max_clock_info(Clock::Graphics))?,
            clock_mem_max_mhz: opt(dev.max_clock_info(Clock::Memory))?,
            temp_slowdown_c: opt(dev.temperature_threshold(TemperatureThreshold::Slowdown))?,
            num_fans: opt(dev.num_fans())?.unwrap_or(0),
        })
    }

    fn utilization(&mut self) -> Result<(u32, u32), Fail> {
        let u = self.dev.utilization_rates().map_err(map_err)?;
        Ok((u.gpu, u.memory))
    }

    fn temperature_c(&mut self) -> Result<u32, Fail> {
        self.dev
            .temperature(TemperatureSensor::Gpu)
            .map_err(map_err)
    }

    fn power_w(&mut self) -> Result<f64, Fail> {
        // mW → W. `power_usage` (the 1 s average) once the instant field has
        // said NotSupported — asked once, like every pruned field.
        if !self.instant_unsupported {
            match self.field_u64(FI_POWER_INSTANT) {
                Ok(mw) => return Ok(mw as f64 / 1000.0),
                Err(Fail::NotSupported) => self.instant_unsupported = true,
                Err(e) => return Err(e),
            }
        }
        Ok(f64::from(self.dev.power_usage().map_err(map_err)?) / 1000.0)
    }

    fn power_limit_w(&mut self) -> Result<f64, Fail> {
        Ok(f64::from(self.dev.enforced_power_limit().map_err(map_err)?) / 1000.0)
    }

    fn clock_gfx_mhz(&mut self) -> Result<u32, Fail> {
        // nvtop prints max(graphics, SM); both are ≈ 0.5 µs.
        let g = self.dev.clock_info(Clock::Graphics).map_err(map_err)?;
        let sm = self.dev.clock_info(Clock::SM).unwrap_or(0);
        Ok(g.max(sm))
    }

    fn clock_mem_mhz(&mut self) -> Result<u32, Fail> {
        self.dev.clock_info(Clock::Memory).map_err(map_err)
    }

    fn pstate(&mut self) -> Result<u8, Fail> {
        self.dev
            .performance_state()
            .map(pstate_num)
            .map_err(map_err)
    }

    fn throttle_bits(&mut self) -> Result<u64, Fail> {
        self.dev
            .current_throttle_reasons()
            .map(|r| r.bits())
            .map_err(map_err)
    }

    fn memory_b(&mut self) -> Result<(u64, u64), Fail> {
        // v2: `used` excludes `reserved` (digest: 13856 vs v1's 14316 MiB).
        let m = self.dev.memory_info().map_err(map_err)?;
        Ok((m.used, m.total))
    }

    fn encoder_pct(&mut self) -> Result<u32, Fail> {
        Ok(self.dev.encoder_utilization().map_err(map_err)?.utilization)
    }

    fn decoder_pct(&mut self) -> Result<u32, Fail> {
        Ok(self.dev.decoder_utilization().map_err(map_err)?.utilization)
    }

    fn pcie_link(&mut self) -> Result<(u32, u32), Fail> {
        let dev = &self.dev;
        Ok((
            dev.current_pcie_link_gen().map_err(map_err)?,
            dev.current_pcie_link_width().map_err(map_err)?,
        ))
    }

    fn pcie_bytes(&mut self) -> Result<(u64, u64), Fail> {
        // One batched call for both counters (0.45 ms each alone).
        let values = self
            .dev
            .field_values_for(&[FieldId(FI_PCIE_TX_BYTES), FieldId(FI_PCIE_RX_BYTES)])
            .map_err(map_err)?;
        let mut it = values.into_iter();
        let mut next = || -> Result<u64, Fail> {
            let sample = it.next().ok_or(Fail::NotSupported)?.map_err(map_err)?;
            match sample.value.map_err(map_err)? {
                SampleValue::U64(v) => Ok(v),
                SampleValue::U32(v) => Ok(u64::from(v)),
                SampleValue::I64(v) => Ok(v.max(0) as u64),
                SampleValue::F64(v) => Ok(v.max(0.0) as u64),
            }
        };
        Ok((next()?, next()?))
    }

    fn fan_pct(&mut self, fan: u32) -> Result<u32, Fail> {
        self.dev.fan_speed(fan).map_err(map_err)
    }

    fn fan_rpm(&mut self, fan: u32) -> Result<u32, Fail> {
        self.dev.fan_speed_rpm(fan).map_err(map_err)
    }

    fn power_samples(&mut self, last_ts: u64) -> Result<Vec<(u64, f32)>, Fail> {
        let samples = self
            .dev
            .samples(
                Sampling::Power,
                if last_ts == 0 { None } else { Some(last_ts) },
            )
            .map_err(map_err)?;
        let mut out: Vec<(u64, f32)> = samples
            .into_iter()
            .map(|s| {
                let mw = match s.value {
                    SampleValue::U32(v) => f64::from(v),
                    SampleValue::U64(v) => v as f64,
                    SampleValue::I64(v) => v as f64,
                    SampleValue::F64(v) => v,
                };
                (s.timestamp, (mw / 1000.0) as f32)
            })
            .collect();
        out.sort_by_key(|(t, _)| *t);
        Ok(out)
    }

    fn graphics_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
        Self::procs(self.dev.running_graphics_processes())
    }

    fn compute_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
        Self::procs(self.dev.running_compute_processes())
    }

    fn proc_util(&mut self, last_seen_us: u64) -> Result<Vec<ProcUtil>, Fail> {
        Ok(self
            .dev
            .process_utilization_stats(Some(last_seen_us))
            .map_err(map_err)?
            .into_iter()
            .map(|s| ProcUtil {
                pid: s.pid,
                timestamp_us: s.timestamp,
                sm: s.sm_util,
                mem: s.mem_util,
                enc: s.enc_util,
                dec: s.dec_util,
            })
            .collect())
    }
}
