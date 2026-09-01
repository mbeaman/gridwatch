//! sysfs reads for the cpu source (§8): CPU topology, per-core frequency and
//! k10temp temperatures **by label** (torch has no `temp2`, so indices are
//! never assumed contiguous — research digest, verified 2026-08-30).

use std::path::{Path, PathBuf};

use gridwatch_store::keys::cpu::CpuTopology;

/// Read a sysfs file and parse it, ignoring every failure (a missing file is a
/// normal degraded state, not an error worth a status).
fn read_num<T: std::str::FromStr>(path: &Path) -> Option<T> {
    std::fs::read_to_string(path).ok()?.trim().parse::<T>().ok()
}

fn read_str(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Number of logical CPUs, from the count of `cpu<N>` directories.
pub fn cpu_count(sys: &Path) -> usize {
    let dir = sys.join("devices/system/cpu");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("cpu"))
                .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        })
        .count()
}

/// The die/core map (§8, D43). Single socket is assumed — `die_id` falls back
/// to `physical_package_id` and then 0, which keeps one block on machines that
/// expose neither.
pub fn topology(sys: &Path, cpus: usize) -> CpuTopology {
    let mut die_of = Vec::with_capacity(cpus);
    let mut core_of = Vec::with_capacity(cpus);
    for cpu in 0..cpus {
        let t = sys.join(format!("devices/system/cpu/cpu{cpu}/topology"));
        let die = read_num::<u16>(&t.join("die_id"))
            .or_else(|| read_num::<u16>(&t.join("physical_package_id")))
            .unwrap_or(0);
        let core = read_num::<u16>(&t.join("core_id")).unwrap_or(cpu as u16);
        die_of.push(die);
        core_of.push(core);
    }
    let dies = die_of
        .iter()
        .copied()
        .max()
        .map(|m| m as usize + 1)
        .unwrap_or(0);
    CpuTopology {
        die_of,
        core_of,
        die_temp: Vec::with_capacity(dies),
    }
}

/// `scaling_cur_freq` paths per logical CPU, resolved once (2 ms for 32 reads).
pub fn freq_paths(sys: &Path, cpus: usize) -> Vec<PathBuf> {
    (0..cpus)
        .map(|c| {
            sys.join(format!(
                "devices/system/cpu/cpu{c}/cpufreq/scaling_cur_freq"
            ))
        })
        .collect()
}

pub fn freq_mhz(path: &Path) -> Option<f64> {
    read_num::<u64>(path).map(|khz| khz as f64 / 1000.0)
}

/// One resolved temperature input: the `chip:label` key name and its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TempInput {
    pub label: String,
    pub path: PathBuf,
}

/// Resolve k10temp's inputs **by label** (`Tctl`, `Tccd1`, `Tccd2`): torch's
/// k10temp exposes temp1/temp3/temp4 with no temp2, so scanning indices or
/// assuming contiguity reads the wrong sensor (research digest §5).
pub fn temp_inputs(sys: &Path, chip: &str) -> Vec<TempInput> {
    let hwmon = sys.join("class/hwmon");
    let Ok(entries) = std::fs::read_dir(&hwmon) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    let mut out = Vec::new();
    for dir in dirs {
        if read_str(&dir.join("name")).as_deref() != Some(chip) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut labels: Vec<(u32, String, PathBuf)> = Vec::new();
        for f in files.flatten() {
            let name = f.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(idx) = name
                .strip_prefix("temp")
                .and_then(|r| r.strip_suffix("_label"))
            else {
                continue;
            };
            // Sort by the *numeric* index: torch's k10temp is temp1/temp3/temp4
            // (no temp2), and lexical order would reshuffle a ten-input chip.
            let Ok(order) = idx.parse::<u32>() else {
                continue;
            };
            let Some(label) = read_str(&f.path()) else {
                continue;
            };
            let input = dir.join(format!("temp{idx}_input"));
            if input.exists() {
                labels.push((order, label, input));
            }
        }
        labels.sort();
        out.extend(labels.into_iter().map(|(_, label, path)| TempInput {
            label: format!("{chip}:{label}"),
            path,
        }));
    }
    out
}

/// Millidegrees → °C.
pub fn temp_c(path: &Path) -> Option<f64> {
    read_num::<i64>(path).map(|milli| milli as f64 / 1000.0)
}

/// Digits of `pid_max`, clamped 5..=19 — the PID column's width (§8).
pub fn pid_digits(proc_root: &Path) -> u8 {
    let max = read_num::<u64>(&proc_root.join("sys/kernel/pid_max")).unwrap_or(32768);
    (max.to_string().len() as u8).clamp(5, 19)
}

/// Live process count: one `readdir` of `/proc`, no per-pid file is opened —
/// the pid-level scan itself is `Detail::Table` work and lands in arc 2 (§8.1).
pub fn process_count(proc_root: &Path) -> Option<u64> {
    let entries = std::fs::read_dir(proc_root).ok()?;
    Some(
        entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .count() as u64,
    )
}
