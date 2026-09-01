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
/// Class breakdown record: unlabelled is `/proc/stat`'s aggregate `cpu` line
/// (what htop's CPU meter draws); `.idx(n)` is core *n*.
pub const BREAKDOWN: Key<CoreBreakdown> = Key::new("cpu.breakdown");

pub const MEM_TOTAL_B: Key<f64> = Key::new("mem.total_b");
pub const MEM_USED_B: Key<f64> = Key::new("mem.used_b");
pub const MEM_AVAILABLE_B: Key<f64> = Key::new("mem.available_b");
pub const MEM_CACHED_B: Key<f64> = Key::new("mem.cached_b");
pub const MEM_BUFFERS_B: Key<f64> = Key::new("mem.buffers_b");
pub const MEM_SHARED_B: Key<f64> = Key::new("mem.shared_b");
pub const SWAP_TOTAL_B: Key<f64> = Key::new("swap.total_b");
pub const SWAP_USED_B: Key<f64> = Key::new("swap.used_b");
pub const SWAP_CACHED_B: Key<f64> = Key::new("swap.cached_b");

pub const PSI_CPU: Key<f64> = Key::new("psi.cpu");
pub const PSI_MEM: Key<f64> = Key::new("psi.mem");
pub const PSI_IO: Key<f64> = Key::new("psi.io");

/// Temperatures: use `.named(&label)` with `chip:label` (e.g. `k10temp:Tccd1`).
/// Produced by the cpu source until the sensors source takes the key over (§8).
pub const TEMP_C: Key<f64> = Key::new("sensor.temp_c");

/// The process table record (arc 2 producer). Latest-only (§4.1).
pub const PROC_TABLE: Key<ProcTable> = Key::new("proc.table");

/// CPU die/core map, published once per source generation (§8, D43). Latest-only.
pub const TOPOLOGY: Key<CpuTopology> = Key::new("cpu.topology");

/// htop's per-class shares for one core's bar (§8; fractions of the period).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreBreakdown {
    pub nice: f32,
    pub user: f32,
    pub kernel: f32,
    pub virt: f32,
    pub iowait: f32,
}

/// The die/core map behind the `cores` tier (D43): which logical CPUs share a
/// CCD and which are SMT siblings. Read from sysfs `topology/{die_id,core_id}`
/// by the cpu source, because a component may not touch a device (§4.6) and the
/// map is not derivable from cpu numbering (torch: CCD0 = cpu0–7 + 16–23).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuTopology {
    /// Die (CCD) id per logical CPU, indexed by cpu id.
    pub die_of: Vec<u16>,
    /// Physical core id per logical CPU, indexed by cpu id.
    pub core_of: Vec<u16>,
    /// `sensor.temp_c` label per die (`k10temp:Tccd1`); empty when unknown.
    pub die_temp: Vec<String>,
}

impl CpuTopology {
    pub fn is_empty(&self) -> bool {
        self.die_of.is_empty() || self.die_of.len() != self.core_of.len()
    }

    /// Dies in ascending id order, each as its physical cores in ascending core
    /// id order, each as its logical CPUs (the SMT pair) in ascending order.
    pub fn dies(&self) -> Vec<(u16, Vec<Vec<u16>>)> {
        use std::collections::BTreeMap;
        let mut by_die: BTreeMap<u16, BTreeMap<u16, Vec<u16>>> = BTreeMap::new();
        for (cpu, (die, core)) in self.die_of.iter().zip(&self.core_of).enumerate() {
            by_die
                .entry(*die)
                .or_default()
                .entry(*core)
                .or_default()
                .push(cpu as u16);
        }
        by_die
            .into_iter()
            .map(|(die, cores)| (die, cores.into_values().collect()))
            .collect()
    }

    /// The temperature label for a die id, if the source resolved one.
    pub fn temp_label(&self, die: u16) -> Option<&str> {
        self.die_temp
            .get(die as usize)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
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

fn decode_topology(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<CpuTopology>(v)
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
        doc: "class shares: nice/user/kernel/virt/iowait; {core} per core, unlabelled = the aggregate line",
        decode: Some(decode_breakdown),
    },
    scalar!("mem.total_b", Bytes, "MemTotal"),
    scalar!(
        "mem.used_b",
        Bytes,
        "htop's used = MemTotal − (MemFree + Cached + SReclaimable + Buffers)"
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
    scalar!(
        "swap.used_b",
        Bytes,
        "SwapTotal − SwapFree − SwapCached (htop's usedSwap)"
    ),
    scalar!(
        "swap.cached_b",
        Bytes,
        "SwapCached (htop's SWP cache segment)"
    ),
    scalar!("psi.cpu", Percent, "PSI some avg10, cpu"),
    scalar!("psi.mem", Percent, "PSI some avg10, memory"),
    scalar!("psi.io", Percent, "PSI some avg10, io"),
    scalar!(
        "sensor.temp_c",
        Celsius,
        "temperature {chip:label}; cpu source owns k10temp until arc 5"
    ),
    KeyMeta {
        name: "cpu.topology",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "die/core map: die_of, core_of, per-die temp label; latest-only",
        decode: Some(decode_topology),
    },
    KeyMeta {
        name: "proc.table",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "pid-level process scan (arc 2); latest-only",
        decode: Some(decode_proc_table),
    },
];
