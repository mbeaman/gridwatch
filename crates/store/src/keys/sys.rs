//! System-wide keys produced by the cpu source (§8).

use crate::key::{DatumKind, Key, KeyMeta, Unit};
use crate::source::SourceId;

pub const SOURCE: SourceId = SourceId("cpu");

pub const LOAD1: Key<f64> = Key::new("sys.load1");
pub const LOAD5: Key<f64> = Key::new("sys.load5");
pub const LOAD15: Key<f64> = Key::new("sys.load15");
pub const UPTIME_S: Key<f64> = Key::new("sys.uptime_s");
pub const PID_DIGITS: Key<f64> = Key::new("sys.pid_digits");
pub const TASKS_TOTAL: Key<f64> = Key::new("tasks.total");
pub const TASKS_THREADS: Key<f64> = Key::new("tasks.threads");
pub const TASKS_RUNNING: Key<f64> = Key::new("tasks.running");
pub const TASKS_KERNEL: Key<f64> = Key::new("tasks.kernel");
/// Wall time of the last pid-level scan, in milliseconds — P15's evidence,
/// shown by the `sources` tile. Published only when a scan ran.
pub const SCAN_MS: Key<f64> = Key::new("sys.scan_ms");

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
    meta!("sys.load1", Ratio, "1-minute load average"),
    meta!("sys.load5", Ratio, "5-minute load average"),
    meta!("sys.load15", Ratio, "15-minute load average"),
    meta!("sys.uptime_s", Seconds, "system uptime"),
    meta!(
        "sys.pid_digits",
        Count,
        "digits of /proc/sys/kernel/pid_max, clamped 5..19 (PID column width)"
    ),
    meta!(
        "tasks.total",
        Count,
        "pid directories in /proc — every process incl. kernel threads; htop's \
         Tasks meter excludes them and needs the arc-2 scan to do so"
    ),
    meta!(
        "tasks.threads",
        Count,
        "all tasks from /proc/loadavg (kernel threads included); htop's \
         userland-thread count needs the arc-2 scan"
    ),
    meta!(
        "tasks.running",
        Count,
        "runnable tasks (/proc/stat procs_running)"
    ),
    meta!(
        "tasks.kernel",
        Count,
        "kernel threads (PF_KTHREAD per pid); only while the pid-level scan runs (Detail::Table)"
    ),
    meta!(
        "sys.scan_ms",
        Milliseconds,
        "wall milliseconds of the last pid-level /proc scan (P15 evidence); only while it runs"
    ),
];
