//! The cpu sampler (§8, brief 1b task 1): htop 3.4.1's formulas verbatim over
//! procfs 0.18, with every root parametrised so `fixtures/procfs/` can drive
//! the same code the live source runs.
//!
//! htop's semantics that matter here (`LinuxMachine.c`, verified at tag 3.4.1):
//! `user -= guest`, `nice -= guest_nice` (the kernel double-counts guest inside
//! user), `systemall = system + irq + softirq`, `idleall = idle + iowait`,
//! `virt = guest + guest_nice`, every delta is a `saturating_sub`, and the
//! meter's four segments are nice / user / kernel / virt — **iowait is not
//! drawn**, it counts as idle (htop's default, `detailed_cpu_time = 0`).

use std::path::PathBuf;
use std::sync::Arc;

use gridwatch_store::keys::cpu::CoreBreakdown;
use gridwatch_store::keys::{cpu, sys};
use gridwatch_store::{Datum, Detail, Key, Label, MetricId, Sample, SourceError, Ts};
use procfs::prelude::*;
use procfs::{CpuPressure, CpuTime, IoPressure, KernelStats, LoadAverage, Meminfo, MemoryPressure};

use super::sysfs;

/// One CPU line of `/proc/stat` after htop's guest correction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticks {
    pub user: u64,
    pub nice: u64,
    pub systemall: u64,
    pub idle: u64,
    pub iowait: u64,
    pub steal: u64,
    pub virt: u64,
}

impl Ticks {
    pub fn from_cpu_time(t: &CpuTime) -> Ticks {
        let guest = t.guest.unwrap_or(0);
        let guest_nice = t.guest_nice.unwrap_or(0);
        Ticks {
            // The kernel counts guest inside user and guest_nice inside nice;
            // htop subtracts both before anything else.
            user: t.user.saturating_sub(guest),
            nice: t.nice.saturating_sub(guest_nice),
            systemall: t.system + t.irq.unwrap_or(0) + t.softirq.unwrap_or(0),
            idle: t.idle,
            iowait: t.iowait.unwrap_or(0),
            steal: t.steal.unwrap_or(0),
            virt: guest + guest_nice,
        }
    }

    /// htop's `total`: user + nice + systemall + idleall + steal + virt.
    pub fn total(self) -> u64 {
        self.user + self.nice + self.systemall + self.idle + self.iowait + self.steal + self.virt
    }
}

/// htop's per-class shares over one period, as fractions of the period.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shares {
    pub nice: f32,
    pub user: f32,
    pub kernel: f32,
    /// steal + guest: htop's non-detailed meter draws them as one segment.
    pub virt: f32,
    /// Not drawn by the meter (it counts as idle); published for the record.
    pub iowait: f32,
}

impl Shares {
    /// The busy fraction the meter shows: everything but idle and iowait.
    pub fn busy(self) -> f32 {
        self.nice + self.user + self.kernel + self.virt
    }
}

/// Parse a `/proc/stat` text into htop's tick vectors: the aggregate line, one
/// per CPU, and `procs_running`. Separated from the read so fixtures exercise
/// exactly the code the live source runs.
pub fn parse_stat(text: &str) -> Result<(Ticks, Vec<Ticks>, Option<u32>), SourceError> {
    let stats =
        KernelStats::from_read(text.as_bytes(), procfs::current_system_info()).map_err(|e| {
            SourceError {
                reason: format!("/proc/stat: {e}"),
                hint: Some("procfs must be mounted at /proc".into()),
            }
        })?;
    Ok((
        Ticks::from_cpu_time(&stats.total),
        stats.cpu_time.iter().map(Ticks::from_cpu_time).collect(),
        stats.procs_running,
    ))
}

/// htop's delta arithmetic: `saturating_sub` per field, share of the period.
pub fn shares(prev: Ticks, cur: Ticks) -> Shares {
    let d = |a: u64, b: u64| a.saturating_sub(b) as f32;
    let period = cur.total().saturating_sub(prev.total()).max(1) as f32;
    Shares {
        nice: d(cur.nice, prev.nice) / period,
        user: d(cur.user, prev.user) / period,
        kernel: d(cur.systemall, prev.systemall) / period,
        virt: (d(cur.steal, prev.steal) + d(cur.virt, prev.virt)) / period,
        iowait: d(cur.iowait, prev.iowait) / period,
    }
}

/// Where the sampler reads from; the live source uses `/proc` and `/sys`.
#[derive(Clone, Debug)]
pub struct Roots {
    pub proc: PathBuf,
    pub sys: PathBuf,
}

impl Default for Roots {
    fn default() -> Roots {
        Roots {
            proc: PathBuf::from("/proc"),
            sys: PathBuf::from("/sys"),
        }
    }
}

fn scalar(key: &Key<f64>, v: f64) -> Sample {
    Sample {
        id: key.id.clone(),
        datum: Datum::Scalar(v),
    }
}

/// The pure, thread-free half of the cpu source (§4.3 `Sampler`): every read is
/// best-effort except `/proc/stat`, whose absence is the one real error.
#[derive(Debug)]
pub struct CpuSampler {
    roots: Roots,
    prev_total: Option<Ticks>,
    prev_cores: Vec<Ticks>,
    cpus: usize,
    freq_paths: Vec<PathBuf>,
    temps: Vec<sysfs::TempInput>,
    topology: cpu::CpuTopology,
    topology_sent: bool,
    probed: bool,
}

impl CpuSampler {
    pub fn new(roots: Roots) -> CpuSampler {
        CpuSampler {
            roots,
            prev_total: None,
            prev_cores: Vec::new(),
            cpus: 0,
            freq_paths: Vec::new(),
            temps: Vec::new(),
            topology: cpu::CpuTopology::default(),
            topology_sent: false,
            probed: false,
        }
    }

    pub fn topology(&self) -> &cpu::CpuTopology {
        &self.topology
    }

    /// Re-point the `/proc` root, keeping the previous scan — how the fixture
    /// tests feed two recorded ticks through one sampler.
    pub fn with_proc_root(mut self, proc: PathBuf) -> CpuSampler {
        self.roots.proc = proc;
        self
    }

    /// One-time sysfs probe: topology, freq paths, k10temp inputs by label.
    fn probe(&mut self, cpus_from_stat: usize) {
        if self.probed {
            return;
        }
        self.probed = true;
        self.cpus = match sysfs::cpu_count(&self.roots.sys) {
            0 => cpus_from_stat, // no sysfs (fixture root, container): trust /proc/stat
            n => n,
        };
        self.freq_paths = sysfs::freq_paths(&self.roots.sys, self.cpus);
        self.temps = sysfs::temp_inputs(&self.roots.sys, "k10temp");
        let mut topo = sysfs::topology(&self.roots.sys, self.cpus);
        // One Tccd label per die in ascending order — htop's CCD attribution,
        // but over the real `die_id` map instead of its cores-per-CCD guess.
        // `temp_inputs` already returns the chip's inputs in numeric index
        // order; re-sorting the labels lexically would put Tccd10 before Tccd2.
        let tccd: Vec<String> = self
            .temps
            .iter()
            .map(|t| t.label.clone())
            .filter(|l| l.contains("Tccd"))
            .collect();
        let dies = topo
            .die_of
            .iter()
            .copied()
            .max()
            .map(|m| m as usize + 1)
            .unwrap_or(0);
        topo.die_temp = (0..dies)
            .map(|i| tccd.get(i).cloned().unwrap_or_default())
            .collect();
        self.topology = topo;
    }

    /// `/proc/stat` → aggregate + per-core percentages and breakdown records.
    /// The first tick has no previous scan, so it emits no percentage at all —
    /// the component renders `—`, never a fabricated 0 (brief 1b gotchas).
    fn cpu_samples(&mut self, out: &mut Vec<Sample>) -> Result<(), SourceError> {
        let path = self.roots.proc.join("stat");
        let text = std::fs::read_to_string(&path).map_err(|e| SourceError {
            reason: format!("{}: {e}", path.display()),
            hint: Some("procfs must be mounted at /proc".into()),
        })?;
        let (total, cores, procs_running) = parse_stat(&text)?;
        self.probe(cores.len());

        if let Some(prev) = self.prev_total {
            let s = shares(prev, total);
            out.push(scalar(&cpu::TOTAL_PCT, f64::from(s.busy()) * 100.0));
            // The unlabelled breakdown is the aggregate `cpu` line — the four
            // segments htop's CPU meter draws.
            out.push(Sample {
                id: cpu::BREAKDOWN.id.clone(),
                datum: Datum::Record(Arc::new(CoreBreakdown {
                    nice: s.nice,
                    user: s.user,
                    kernel: s.kernel,
                    virt: s.virt,
                    iowait: s.iowait,
                })),
            });
        }
        for (i, cur) in cores.iter().enumerate() {
            let Some(prev) = self.prev_cores.get(i).copied() else {
                continue;
            };
            let s = shares(prev, *cur);
            out.push(scalar(
                &cpu::CORE_PCT.idx(i as u16),
                f64::from(s.busy()) * 100.0,
            ));
            out.push(Sample {
                id: cpu::BREAKDOWN.idx(i as u16).id,
                datum: Datum::Record(Arc::new(CoreBreakdown {
                    nice: s.nice,
                    user: s.user,
                    kernel: s.kernel,
                    virt: s.virt,
                    iowait: s.iowait,
                })),
            });
        }
        // procs_running is htop's "running" when no scan is available.
        if let Some(running) = procs_running {
            out.push(scalar(&sys::TASKS_RUNNING, f64::from(running)));
        }
        self.prev_total = Some(total);
        self.prev_cores = cores;
        Ok(())
    }

    /// htop's memory formulas (`LinuxMachine_scanMemoryInfo`), all in bytes.
    fn mem_samples(&self, out: &mut Vec<Sample>) {
        let Ok(m) = Meminfo::from_file(self.roots.proc.join("meminfo")) else {
            return;
        };
        let s_reclaimable = m.s_reclaimable.unwrap_or(0);
        let shmem = m.shmem.unwrap_or(0);
        // cached = Cached + SReclaimable − Shmem; Shmem is drawn as its own
        // segment, so the meter's segments still sum to total − free.
        let cached = (m.cached + s_reclaimable).saturating_sub(shmem);
        // htop falls back to `MemTotal − MemFree` when the parts overshoot the
        // total (it cannot happen on a normal kernel, but htop guards it).
        let parts = m.mem_free + m.cached + s_reclaimable + m.buffers;
        let used = if parts > m.mem_total {
            m.mem_total.saturating_sub(m.mem_free)
        } else {
            m.mem_total - parts
        };
        let available = m.mem_available.unwrap_or(m.mem_free).min(m.mem_total);
        out.push(scalar(&cpu::MEM_TOTAL_B, m.mem_total as f64));
        out.push(scalar(&cpu::MEM_USED_B, used as f64));
        out.push(scalar(&cpu::MEM_AVAILABLE_B, available as f64));
        out.push(scalar(&cpu::MEM_CACHED_B, cached as f64));
        out.push(scalar(&cpu::MEM_BUFFERS_B, m.buffers as f64));
        out.push(scalar(&cpu::MEM_SHARED_B, shmem as f64));
        out.push(scalar(&cpu::SWAP_TOTAL_B, m.swap_total as f64));
        out.push(scalar(
            &cpu::SWAP_USED_B,
            m.swap_total.saturating_sub(m.swap_free + m.swap_cached) as f64,
        ));
        out.push(scalar(&cpu::SWAP_CACHED_B, m.swap_cached as f64));
    }

    /// loadavg, uptime, task counts and the PID column width.
    fn sys_samples(&self, out: &mut Vec<Sample>) {
        if let Ok(l) = LoadAverage::from_file(self.roots.proc.join("loadavg")) {
            out.push(scalar(&sys::LOAD1, f64::from(l.one)));
            out.push(scalar(&sys::LOAD5, f64::from(l.five)));
            out.push(scalar(&sys::LOAD15, f64::from(l.fifteen)));
            // `cur/max` is runnable/total *tasks* (threads), not processes.
            out.push(scalar(&sys::TASKS_THREADS, f64::from(l.max)));
        }
        if let Ok(text) = std::fs::read_to_string(self.roots.proc.join("uptime"))
            && let Some(secs) = text
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
        {
            out.push(scalar(&sys::UPTIME_S, secs));
        }
        if let Some(n) = sysfs::process_count(&self.roots.proc) {
            out.push(scalar(&sys::TASKS_TOTAL, n as f64));
        }
        // `tasks.kernel` needs the pid-level scan (PF_KTHREAD per pid) and is
        // therefore arc 2 work; the tile renders `—` until then (PARITY.md).
        out.push(scalar(
            &sys::PID_DIGITS,
            f64::from(sysfs::pid_digits(&self.roots.proc)),
        ));
    }

    /// PSI `some avg10` for cpu / memory / io. torch has no `pressure/irq`
    /// (CONFIG_IRQ_TIME_ACCOUNTING is off), so it is never read.
    fn psi_samples(&self, out: &mut Vec<Sample>) {
        let dir = self.roots.proc.join("pressure");
        if let Ok(p) = CpuPressure::from_file(dir.join("cpu")) {
            out.push(scalar(&cpu::PSI_CPU, f64::from(p.some.avg10)));
        }
        if let Ok(p) = MemoryPressure::from_file(dir.join("memory")) {
            out.push(scalar(&cpu::PSI_MEM, f64::from(p.some.avg10)));
        }
        if let Ok(p) = IoPressure::from_file(dir.join("io")) {
            out.push(scalar(&cpu::PSI_IO, f64::from(p.some.avg10)));
        }
    }

    /// Per-core frequency (32 reads ≈ 2 ms on torch) and k10temp by label.
    fn sysfs_samples(&self, out: &mut Vec<Sample>) {
        for (i, path) in self.freq_paths.iter().enumerate() {
            if let Some(mhz) = sysfs::freq_mhz(path) {
                out.push(scalar(&cpu::FREQ_MHZ.idx(i as u16), mhz));
            }
        }
        for t in &self.temps {
            if let Some(c) = sysfs::temp_c(&t.path) {
                out.push(Sample {
                    id: MetricId {
                        name: cpu::TEMP_C.id.name,
                        label: Label::Name(Arc::from(t.label.as_str())),
                    },
                    datum: Datum::Scalar(c),
                });
            }
        }
    }
}

impl gridwatch_store::Sampler for CpuSampler {
    fn sample(&mut self, _now: Ts, detail: Detail) -> Result<Vec<Sample>, SourceError> {
        let mut out = Vec::with_capacity(self.cpus * 3 + 32);
        self.cpu_samples(&mut out)?;
        self.mem_samples(&mut out);
        self.sys_samples(&mut out);
        self.psi_samples(&mut out);
        self.sysfs_samples(&mut out);
        if !self.topology_sent && !self.topology.is_empty() {
            self.topology_sent = true;
            out.push(Sample {
                id: cpu::TOPOLOGY.id.clone(),
                datum: Datum::Record(Arc::new(self.topology.clone())),
            });
        }
        // `Detail::Table`/`Columns` add the pid-level scan and htop's gated
        // files in arc 2 (§8.1); 1b is meters-only whatever the demand says.
        let _ = detail;
        Ok(out)
    }
}
