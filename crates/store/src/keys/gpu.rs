//! GPU keys (§8, §8.1, brief arc 2 seam 4): every key is labelled `{dev}`
//! (`.idx(dev)`), fans `{dev:i}` (`.named("0:1")`). The Record shapes here are
//! the journal seam — a change is a DECISIONS entry.

use std::borrow::Cow;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit, Vec32};
use crate::source::SourceId;

pub const SOURCE: SourceId = SourceId("gpu");

/// `utilization_rates().gpu` — SM busy percentage.
pub const UTIL_PCT: Key<f64> = Key::new("gpu.util_pct");
/// `utilization_rates().memory` — memory-controller busy percentage. *Not*
/// VRAM occupancy (nvtop's MEM bar is `vram_used_b / vram_total_b`).
pub const MEMCTL_PCT: Key<f64> = Key::new("gpu.memctl_pct");
pub const VRAM_USED_B: Key<f64> = Key::new("gpu.vram_used_b");
pub const VRAM_TOTAL_B: Key<f64> = Key::new("gpu.vram_total_b");
pub const POWER_W: Key<f64> = Key::new("gpu.power_w");
pub const POWER_LIMIT_W: Key<f64> = Key::new("gpu.power_limit_w");
/// The 20 ms board-power trace (`samples(Power)`), watts, oldest first — one
/// vector per slow tick (the store's Vector series keep 64 of them).
pub const POWER_TRACE: Key<Vec32> = Key::new("gpu.power_trace");
pub const TEMP_C: Key<f64> = Key::new("gpu.temp_c");
pub const TEMP_SLOWDOWN_C: Key<f64> = Key::new("gpu.temp_slowdown_c");
/// Fan setpoint, per fan: `.named("0:1")` is device 0, fan 1.
pub const FAN_PCT: Key<f64> = Key::new("gpu.fan_pct");
pub const FAN_RPM: Key<f64> = Key::new("gpu.fan_rpm");
pub const CLOCK_GFX_MHZ: Key<f64> = Key::new("gpu.clock_gfx_mhz");
pub const CLOCK_MEM_MHZ: Key<f64> = Key::new("gpu.clock_mem_mhz");
pub const CLOCK_GFX_MAX_MHZ: Key<f64> = Key::new("gpu.clock_gfx_max_mhz");
pub const CLOCK_MEM_MAX_MHZ: Key<f64> = Key::new("gpu.clock_mem_max_mhz");
pub const PCIE_RX_BPS: Key<f64> = Key::new("gpu.pcie_rx_bps");
pub const PCIE_TX_BPS: Key<f64> = Key::new("gpu.pcie_tx_bps");
pub const PCIE_GEN: Key<f64> = Key::new("gpu.pcie_gen");
pub const PCIE_WIDTH: Key<f64> = Key::new("gpu.pcie_width");
pub const ENC_PCT: Key<f64> = Key::new("gpu.enc_pct");
pub const DEC_PCT: Key<f64> = Key::new("gpu.dec_pct");
/// Performance state 0–15; 32 = unknown.
pub const PSTATE: Key<f64> = Key::new("gpu.pstate");
pub const THROTTLE: Key<Throttle> = Key::new("gpu.throttle");
pub const INFO: Key<GpuInfo> = Key::new("gpu.info");
pub const PROCS: Key<GpuProcs> = Key::new("gpu.procs");
/// NVML wall time per second per call class (`{fast}`, `{slow}`, `{procs}`):
/// P11's evidence, shown by the `sources` tile (D49). Published on the slow
/// tier; a class that did not run in the window publishes 0.
pub const NVML_MS: Key<f64> = Key::new("gpu.nvml_ms");

/// The P-state value published when NVML reports `Unknown`.
pub const PSTATE_UNKNOWN: f64 = 32.0;

/// The fan label `{dev:i}`.
pub fn fan_label(dev: u16, fan: u16) -> Arc<str> {
    Arc::from(format!("{dev}:{fan}"))
}

/// Current clock-throttle reasons as NVML's raw bitmask (§8). Bits, per
/// `nvml.h`: 0x01 GpuIdle, 0x02 ApplicationsClocksSetting, 0x04 SwPowerCap,
/// 0x08 HwSlowdown, 0x10 SyncBoost, 0x20 SwThermalSlowdown,
/// 0x40 HwThermalSlowdown, 0x80 HwPowerBrakeSlowdown, 0x100 DisplayClockSetting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Throttle {
    pub bits: u64,
}

impl Throttle {
    pub const GPU_IDLE: u64 = 0x01;
    pub const APP_CLOCKS: u64 = 0x02;
    pub const SW_POWER_CAP: u64 = 0x04;
    pub const HW_SLOWDOWN: u64 = 0x08;
    pub const SYNC_BOOST: u64 = 0x10;
    pub const SW_THERMAL: u64 = 0x20;
    pub const HW_THERMAL: u64 = 0x40;
    pub const HW_POWER_BRAKE: u64 = 0x80;
    pub const DISPLAY_CLOCKS: u64 = 0x100;

    /// Reasons worth a chip — everything but idle and the two clock settings.
    pub fn is_limiting(self) -> bool {
        self.bits
            & (Self::SW_POWER_CAP
                | Self::HW_SLOWDOWN
                | Self::SW_THERMAL
                | Self::HW_THERMAL
                | Self::HW_POWER_BRAKE
                | Self::SYNC_BOOST)
            != 0
    }

    /// Short labels for the chip, most severe first.
    pub fn labels(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (bit, name) in [
            (Self::HW_POWER_BRAKE, "BRAKE"),
            (Self::HW_THERMAL, "HW THERM"),
            (Self::HW_SLOWDOWN, "HW SLOW"),
            (Self::SW_THERMAL, "THERM"),
            (Self::SW_POWER_CAP, "PWRCAP"),
            (Self::SYNC_BOOST, "SYNC"),
        ] {
            if self.bits & bit != 0 {
                out.push(name);
            }
        }
        out
    }
}

/// The hand-verified static spec row (GPU-Z's column), keyed by PCI device id
/// in the source's `SPECS` table. What NVML cannot supply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuSpec {
    pub sms: u32,
    pub tmus: u32,
    pub rops: u32,
    pub rt_cores: u32,
    pub tensor_cores: u32,
    pub l2_mb: u32,
    pub base_mhz: u32,
    pub boost_mhz: u32,
    /// Memory data rate, Gbps per pin.
    pub mem_gbps: f32,
    pub bandwidth_gbs: u32,
    pub tdp_w: u32,
    pub die_mm2: u32,
    pub transistors_b: f32,
    /// `Cow` so a `const` table row borrows and the journal revives it owned.
    pub launch: Cow<'static, str>,
}

/// The static probe, published once per source generation (§8). Latest-only.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub driver: String,
    /// CUDA driver version as NVML reports it (13030 = 13.3); 0 when unknown.
    pub cuda: u32,
    pub arch: String,
    pub uuid: String,
    /// PCI device id (the upper 16 bits of NVML's `pci_device_id`): `0x2B85`
    /// is the RTX 5090.
    pub pci_id: u32,
    pub bus_id: String,
    pub vbios: String,
    pub cores: Option<u32>,
    pub bus_width: Option<u32>,
    pub spec: Option<GpuSpec>,
    /// True when NVML and the spec row disagreed on cores or bus width; the
    /// NVML value is shown and the `sources` tile says so.
    #[serde(default)]
    pub spec_mismatch: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuProcKind {
    Graphics,
    Compute,
    /// In both v3 lists — nvtop prints such a PID twice; gridwatch merges.
    Both,
}

/// One GPU process row (§8.1). `fresh` is false when `process_utilization_stats`
/// returned no sample newer than `last_seen` for the PID — the percentages
/// then read 0, as in nvtop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuProcRow {
    pub pid: i32,
    pub kind: GpuProcKind,
    pub vram_b: Option<u64>,
    pub sm_pct: u32,
    pub mem_pct: u32,
    pub enc_pct: u32,
    pub dec_pct: u32,
    pub fresh: bool,
}

/// The GPU process set (§8.1). Latest-only.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuProcs {
    pub rows: Vec<GpuProcRow>,
    pub vram_total_b: u64,
}

fn decode<T: for<'de> Deserialize<'de> + RecordValue>(
    v: serde_json::Value,
) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<T>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

fn decode_throttle(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Throttle>(v)
}

fn decode_info(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<GpuInfo>(v)
}

fn decode_procs(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<GpuProcs>(v)
}

macro_rules! scalar {
    ($name:expr, $unit:ident, $doc:expr) => {
        KeyMeta {
            name: $name,
            unit: Unit::$unit,
            kind: DatumKind::Scalar,
            source: SOURCE,
            doc: $doc,
            decode: None,
        }
    };
}

pub static METAS: &[KeyMeta] = &[
    scalar!(
        "gpu.util_pct",
        Percent,
        "SM utilisation {dev} (utilization_rates.gpu)"
    ),
    scalar!(
        "gpu.memctl_pct",
        Percent,
        "memory-controller utilisation {dev} — not VRAM occupancy"
    ),
    scalar!(
        "gpu.vram_used_b",
        Bytes,
        "VRAM used {dev} (memory_info v2, reserved excluded)"
    ),
    scalar!("gpu.vram_total_b", Bytes, "VRAM total {dev}"),
    scalar!(
        "gpu.power_w",
        Watts,
        "board power {dev} (NVML_FI_DEV_POWER_INSTANT)"
    ),
    scalar!("gpu.power_limit_w", Watts, "enforced power limit {dev}"),
    KeyMeta {
        name: "gpu.power_trace",
        unit: Unit::Watts,
        kind: DatumKind::Vector,
        source: SOURCE,
        doc: "20 ms board-power samples {dev}, oldest first, one vector per slow tick; only while a gpu tile is visible",
        decode: None,
    },
    scalar!("gpu.temp_c", Celsius, "GPU temperature {dev}"),
    scalar!("gpu.temp_slowdown_c", Celsius, "slowdown threshold {dev}"),
    scalar!("gpu.fan_pct", Percent, "fan setpoint {dev:fan}; every 5 s"),
    scalar!("gpu.fan_rpm", Count, "fan RPM {dev:fan}; every 5 s"),
    scalar!("gpu.clock_gfx_mhz", Megahertz, "graphics clock {dev}"),
    scalar!("gpu.clock_mem_mhz", Megahertz, "memory clock {dev}"),
    scalar!(
        "gpu.clock_gfx_max_mhz",
        Megahertz,
        "max graphics clock {dev}; static"
    ),
    scalar!(
        "gpu.clock_mem_max_mhz",
        Megahertz,
        "max memory clock {dev}; static"
    ),
    scalar!(
        "gpu.pcie_rx_bps",
        BytesPerSec,
        "PCIe receive rate {dev}, diffed byte counter (field 198)"
    ),
    scalar!(
        "gpu.pcie_tx_bps",
        BytesPerSec,
        "PCIe transmit rate {dev}, diffed byte counter (field 197)"
    ),
    scalar!("gpu.pcie_gen", Count, "current PCIe generation {dev}"),
    scalar!("gpu.pcie_width", Count, "current PCIe link width {dev}"),
    scalar!("gpu.enc_pct", Percent, "encoder utilisation {dev}"),
    scalar!("gpu.dec_pct", Percent, "decoder utilisation {dev}"),
    scalar!(
        "gpu.pstate",
        Count,
        "performance state {dev}: 0–15, 32 = unknown"
    ),
    KeyMeta {
        name: "gpu.throttle",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "current clock-throttle reasons {dev} as NVML's bitmask; latest-only",
        decode: Some(decode_throttle),
    },
    KeyMeta {
        name: "gpu.info",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "static probe {dev}: name, driver, cuda, arch, uuid, pci id, bus id, vbios, cores, bus width, spec row; once per generation",
        decode: Some(decode_info),
    },
    KeyMeta {
        name: "gpu.procs",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "GPU process rows {dev} (v3 lists merged by PID, utilisation overlaid); only at Detail::Table; latest-only",
        decode: Some(decode_procs),
    },
    scalar!(
        "gpu.nvml_ms",
        Milliseconds,
        "NVML wall ms per second per call class {fast|slow|procs} (P11 evidence)"
    ),
];
