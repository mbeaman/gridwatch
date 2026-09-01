//! CPU / memory / pressure / temperature keys and the process-table record (§8, §8.1).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit};
use crate::source::SourceId;

pub const SOURCE: SourceId = SourceId("cpu");

pub const TOTAL_PCT: Key<f64> = Key::new("cpu.total_pct");
/// Per-core: use `.idx(n)`.
pub const CORE_PCT: Key<f64> = Key::new("cpu.core_pct");
pub const FREQ_MHZ: Key<f64> = Key::new("cpu.freq_mhz");
/// Per-core class breakdown record: use `.idx(n)`.
pub const BREAKDOWN: Key<CoreBreakdown> = Key::new("cpu.breakdown");

pub const MEM_TOTAL_B: Key<f64> = Key::new("mem.total_b");
pub const MEM_USED_B: Key<f64> = Key::new("mem.used_b");
pub const MEM_AVAILABLE_B: Key<f64> = Key::new("mem.available_b");
pub const MEM_CACHED_B: Key<f64> = Key::new("mem.cached_b");
pub const MEM_BUFFERS_B: Key<f64> = Key::new("mem.buffers_b");
pub const MEM_SHARED_B: Key<f64> = Key::new("mem.shared_b");
pub const SWAP_TOTAL_B: Key<f64> = Key::new("swap.total_b");
pub const SWAP_USED_B: Key<f64> = Key::new("swap.used_b");

pub const PSI_CPU: Key<f64> = Key::new("psi.cpu");
pub const PSI_MEM: Key<f64> = Key::new("psi.mem");
pub const PSI_IO: Key<f64> = Key::new("psi.io");

/// Temperatures: use `.named(&label)` with `chip:label` (e.g. `k10temp:Tccd1`).
/// Produced by the cpu source until the sensors source takes the key over (§8).
pub const TEMP_C: Key<f64> = Key::new("sensor.temp_c");

/// The process table record (arc 2 producer). Latest-only (§4.1).
pub const PROC_TABLE: Key<ProcTable> = Key::new("proc.table");

/// htop's per-class shares for one core's bar (§8; fractions of the period).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreBreakdown {
    pub nice: f32,
    pub user: f32,
    pub kernel: f32,
    pub virt: f32,
    pub iowait: f32,
}

/// One row of the pid-level process scan (§8.1). Memory fields are KiB as htop
/// prints them; CPU% is Irix-mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcRow {
    pub pid: i32,
    pub ppid: i32,
    pub tgid: i32,
    pub uid: u32,
    pub user: Arc<str>,
    pub state: char,
    pub pri: i16,
    pub nice: i16,
    pub nlwp: u32,
    pub virt_kib: u64,
    pub res_kib: u64,
    pub shr_kib: u64,
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub time_cs: u64,
    pub starttime: u64,
    pub kthread: bool,
    pub cmdline: Arc<str>,
    pub comm: Arc<str>,
}

/// The scan output (§8.1). `rows` is a plain Vec inside the record — the record
/// itself travels as `Arc<dyn RecordValue>`, so the table still swaps in O(1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcTable {
    pub rows: Vec<ProcRow>,
    pub pid_digits: u8,
}

fn decode_breakdown(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<CoreBreakdown>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

fn decode_proc_table(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<ProcTable>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
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
        "cpu.total_pct",
        Percent,
        "aggregate CPU use, htop semantics"
    ),
    scalar!("cpu.core_pct", Percent, "per-core CPU use {core}"),
    scalar!(
        "cpu.freq_mhz",
        Megahertz,
        "per-core scaling frequency {core}"
    ),
    KeyMeta {
        name: "cpu.breakdown",
        unit: Unit::Ratio,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "per-core class shares {core}: nice/user/kernel/virt/iowait",
        decode: Some(decode_breakdown),
    },
    scalar!("mem.total_b", Bytes, "MemTotal"),
    scalar!(
        "mem.used_b",
        Bytes,
        "used = total − available (bar segments come from the parts)"
    ),
    scalar!("mem.available_b", Bytes, "MemAvailable"),
    scalar!(
        "mem.cached_b",
        Bytes,
        "cached = Cached + SReclaimable − Shmem (htop)"
    ),
    scalar!("mem.buffers_b", Bytes, "Buffers"),
    scalar!("mem.shared_b", Bytes, "Shmem"),
    scalar!("swap.total_b", Bytes, "SwapTotal"),
    scalar!("swap.used_b", Bytes, "SwapTotal − SwapFree"),
    scalar!("psi.cpu", Percent, "PSI some avg10, cpu"),
    scalar!("psi.mem", Percent, "PSI some avg10, memory"),
    scalar!("psi.io", Percent, "PSI some avg10, io"),
    scalar!(
        "sensor.temp_c",
        Celsius,
        "temperature {chip:label}; cpu source owns k10temp until arc 5"
    ),
    KeyMeta {
        name: "proc.table",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "pid-level process scan (arc 2); latest-only",
        decode: Some(decode_proc_table),
    },
];
