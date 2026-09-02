//! astral-watch pin keys (§8, brief arc 3 seam 1): six 12V-2x6 pins, 1-based
//! like the connector, plus the totals, the balance ratio, the read cost and
//! two Records. Nothing from `astral_watch` crosses into the store — the
//! source converts (D50 §2).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit};
use crate::source::SourceId;
use crate::ts::Ts;

pub const SOURCE: SourceId = SourceId("pins");

/// Pins are `1..=6`: `.idx(n)` with the connector's own numbering.
pub const PIN_COUNT: u16 = 6;
pub const AMPS: Key<f64> = Key::new("pins.amps");
pub const VOLTS: Key<f64> = Key::new("pins.volts");
pub const TOTAL_A: Key<f64> = Key::new("pins.total_a");
pub const TOTAL_W: Key<f64> = Key::new("pins.total_w");
/// hi/lo pin current ratio; absent when the lowest pin is ≤ 0.05 A.
pub const BALANCE: Key<f64> = Key::new("pins.balance");
/// Wall ms of the last read — P14's evidence, shown by the `sources` tile.
pub const READ_MS: Key<f64> = Key::new("pins.read_ms");
pub const INFO: Key<PinsInfo> = Key::new("pins.info");
pub const STATE: Key<PinsState> = Key::new("pins.state");

/// astral-watch's constants, verbatim (D50 §4 reads the live ones from
/// `pins.info` and falls back to these).
pub const AMPS_CEILING: f64 = 10.0;
pub const OVERLOAD_A: f64 = 9.2;
pub const IMBALANCE_RATIO: f64 = 1.5;
pub const IMBALANCE_ALARM_PIN_FRAC: f64 = 0.85;
pub const MIN_LOAD_A: f64 = 5.0;
pub const BALANCE_WARN: f64 = 1.33;

/// Alert ids the pins source raises: `pins/<Condition::id()>` (D50 §3).
pub const ALERT_OVERLOAD: &str = "pins/overload";
pub const ALERT_DISCONNECTED: &str = "pins/disconnected";
pub const ALERT_IMBALANCE: &str = "pins/imbalance";
pub const ALERT_IMBALANCE_ADVISORY: &str = "pins/imbalance_advisory";
pub const ALERT_TELEMETRY_LOST: &str = "pins/telemetry_lost";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinsMode {
    #[default]
    I2c,
    Exporter,
    Csv,
}

impl PinsMode {
    pub fn label(self) -> &'static str {
        match self {
            PinsMode::I2c => "i2c",
            PinsMode::Exporter => "exporter",
            PinsMode::Csv => "csv",
        }
    }
}

/// The static probe (once per generation and on every mode or bus change):
/// where the telemetry comes from and the thresholds/policy in force.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PinsInfo {
    pub mode: PinsMode,
    pub bus: Option<u32>,
    pub addr: u16,
    /// The card's PCI address; in `Exporter` mode the endpoint (`host:port`).
    pub pci: String,
    pub model: Option<String>,
    /// `"block"`, `"bytewise"` or `"unknown"` over i2c; `"http"` in `Exporter` mode.
    pub access: String,
    pub interval_ms: u32,
    pub overload_a: f64,
    pub imbalance_ratio: f64,
    pub min_load_a: f64,
    pub confirm: u32,
    pub advisory_confirm: u32,
    pub resolve: u32,
    pub repeat_min: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveCondition {
    /// `Condition::id()`: `overload`, `disconnected`, `imbalance`,
    /// `imbalance_advisory`, `telemetry_lost`.
    pub id: String,
    pub detail: String,
    pub since: Ts,
}

/// Per-sample state: the lifecycle's active set and the telemetry health.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PinsState {
    pub telemetry_lost: bool,
    pub misses: u32,
    pub active: Vec<ActiveCondition>,
    /// The exporter's own `alert_active{condition}` flags when `mode ==
    /// Exporter` — shown as a `svc` chip, never merged into the lifecycle.
    #[serde(default)]
    pub service_active: Vec<String>,
}

fn decode<T: for<'de> Deserialize<'de> + RecordValue>(
    v: serde_json::Value,
) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<T>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

fn decode_info(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<PinsInfo>(v)
}

fn decode_state(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<PinsState>(v)
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
        "pins.amps",
        Amps,
        "per-pin current {pin}, pins 1–6 as on the connector"
    ),
    scalar!("pins.volts", Volts, "per-pin voltage {pin}"),
    scalar!("pins.total_a", Amps, "sum of the six pin currents"),
    scalar!("pins.total_w", Watts, "Σ V·I over the six pins"),
    scalar!(
        "pins.balance",
        Ratio,
        "hi/lo pin current ratio; absent when the lowest pin is ≤ 0.05 A"
    ),
    scalar!(
        "pins.read_ms",
        Milliseconds,
        "wall ms of the last telemetry read (P14 evidence)"
    ),
    KeyMeta {
        name: "pins.info",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "mode (i2c/exporter), bus, pci, model, access path, interval and the astral-watch thresholds/policy in force; once per generation and on change",
        decode: Some(decode_info),
    },
    KeyMeta {
        name: "pins.state",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "telemetry health, miss count, the lifecycle's active conditions and the exporter's own flags; every sample",
        decode: Some(decode_state),
    },
];
