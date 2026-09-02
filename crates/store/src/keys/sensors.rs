//! Sensors keys (§8, brief arc 5 seam 1 / 5b): every hwmon reading by
//! `chip:label`, the chip inventory and the RAPL state as Records. The
//! `sensor.temp_c` name is shared with the cpu source's k10temp handover
//! (§16): the sensors source publishes the same name with the same labels
//! (`k10temp:Tccd1`), so the htop tile's `Tccd` column reads unchanged.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit};
use crate::source::SourceId;

pub const SOURCE: SourceId = SourceId("sensors");

/// `sensor.temp_c{chip:label}` — °C. The same `Key` the cpu source used for
/// k10temp (`keys::cpu::TEMP_C` points here).
pub const TEMP_C: Key<f64> = Key::new("sensor.temp_c");
/// `sensor.max_c{chip:label}` / `sensor.crit_c{chip:label}` — the chip's own
/// thresholds, once per generation when it exports them.
pub const MAX_C: Key<f64> = Key::new("sensor.max_c");
pub const CRIT_C: Key<f64> = Key::new("sensor.crit_c");
pub const FAN_RPM: Key<f64> = Key::new("sensor.fan_rpm");
pub const VOLT_V: Key<f64> = Key::new("sensor.volt_v");
/// `sensor.power_w{chip:label}` — `rapl:package-0` when `energy_uj` is readable.
pub const POWER_W: Key<f64> = Key::new("sensor.power_w");
/// The last hwmon walk's wall ms (the `sources` tile's cost line).
pub const WALK_MS: Key<f64> = Key::new("sensor.walk_ms");
pub const INFO: Key<SensorsInfo> = Key::new("sensor.info");

/// Sentinel thresholds above this many m°C are driver bugs (nvme's
/// `temp2_max = 65261850`) and are dropped.
pub const SENTINEL_MILLI: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaplState {
    /// `energy_uj` readable: `sensor.power_w{rapl:package-0}` is published.
    Ok,
    /// `energy_uj` is 0400 root-only (CVE-2020-8694): the udev rule fixes it.
    RootOnly,
    #[default]
    Absent,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChipInfo {
    /// The hwmon `name` (`k10temp`, `nvme`); a duplicate name gets `name#2`.
    pub name: String,
    /// The hwmon directory (`/sys/class/hwmon/hwmon4`) — not stable across boots.
    pub path: String,
    /// The device the chip hangs off (`nvme0`, `0000:00:18.3`, `8-0051`):
    /// what makes `nvme#2` the *same* drive after a reboot.
    #[serde(default)]
    pub device: String,
    /// The reading kinds the chip exports: `temp`, `fan`, `in`, `power`.
    pub kinds: Vec<String>,
}

/// The inventory, once per generation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorsInfo {
    pub chips: Vec<ChipInfo>,
    pub rapl: RaplState,
}

fn decode_info(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<SensorsInfo>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

macro_rules! meta {
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
    meta!(
        "sensor.temp_c",
        Celsius,
        "hwmon temperature per {chip:label} (k10temp:Tctl, nvme:Composite, spd5118:temp1 …); the cpu source publishes k10temp's when the sensors feature is off (§16)"
    ),
    meta!(
        "sensor.max_c",
        Celsius,
        "the chip's own max threshold per {chip:label}, once per generation (sentinels > 1 000 °C dropped)"
    ),
    meta!(
        "sensor.crit_c",
        Celsius,
        "the chip's own critical threshold per {chip:label}, once per generation"
    ),
    meta!(
        "sensor.fan_rpm",
        Count,
        "hwmon fan speed per {chip:label}, RPM"
    ),
    meta!("sensor.volt_v", Volts, "hwmon voltage per {chip:label}"),
    meta!(
        "sensor.power_w",
        Watts,
        "power per {chip:label}: RAPL package-0 from Δenergy_uj/Δt when readable"
    ),
    meta!(
        "sensor.walk_ms",
        Milliseconds,
        "wall ms of the last hwmon walk (the sources tile's cost line)"
    ),
    KeyMeta {
        name: "sensor.info",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the chip inventory (name, path, kinds) and the RAPL state (ok | root_only | absent), once per generation",
        decode: Some(decode_info),
    },
];
