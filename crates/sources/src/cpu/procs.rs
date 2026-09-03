//! The pid-level process scan (§8, §8.1, P15): `stat`, `statm`, the
//! directory's `st_uid`, and `cmdline` on first sight of a `(pid, starttime)`.
//! htop 3.4.1's `LinuxProcessTable.c` semantics: CPU% is Irix-mode
//! `Δ(utime+stime) / period · 100` with `period = Δ(aggregate total) /
//! active_cpus`, clamped to `active_cpus · 100`; MEM% is `resident /
//! MemTotal`; a kernel thread is `PF_KTHREAD` (0x00200000) in stat field 9;
//! deltas are keyed by `(pid, starttime)` so a reused PID never inherits a
//! stranger's counters. Userland threads are **not** rows: the thread-group
//! leader's `stat` already sums its threads, and the `task/` walk is
//! `Detail::Columns` work (arc 8). No `libc::getpwuid_r`: the uid → name map
//! is read from `passwd` by hand and cached.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gridwatch_store::keys::cpu::{ProcRow, ProcTable};
use procfs::prelude::*;
use procfs::process::{Stat, StatM};

const PF_KTHREAD: u32 = 0x0020_0000;

/// What one pass produced.
#[derive(Clone, Debug)]
pub struct Scan {
    pub table: ProcTable,
    /// Rows flagged `PF_KTHREAD` — htop's `kthr`.
    pub kernel_threads: u64,
    /// Wall time of the pass (P15's number).
    pub ms: f64,
}

type Ident = (i32, u64);

#[derive(Debug)]
pub struct ProcScanner {
    proc_root: PathBuf,
    passwd: PathBuf,
    page_kib: u64,
    /// `utime + stime` at the previous pass, per `(pid, starttime)`.
    prev: HashMap<Ident, u64>,
    /// `cmdline` and `comm` as first seen — never re-read (htop's default).
    names: HashMap<Ident, (Arc<str>, Arc<str>)>,
    users: HashMap<u32, Arc<str>>,
    users_loaded: bool,
    prev_total: Option<u64>,
    /// `read_bytes`/`write_bytes` at the previous `Detail::Columns` pass,
    /// and when that pass ran — the I/O screen's rates (arc 8a).
    prev_io: HashMap<Ident, (u64, u64)>,
    io_at: Option<Instant>,
}

/// `/proc/<pid>/io`'s two counters, or `None` when it is not ours to read
/// (EACCES for another user's process — the normal case).
fn read_io(dir: &std::path::Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(dir.join("io")).ok()?;
    let mut read = None;
    let mut write = None;
    for line in text.lines() {
        // `read_bytes` and `write_bytes` are the block-layer counters —
        // `rchar`/`wchar` count anything that passed through a syscall,
        // including a pipe, which is not what an I/O screen means.
        if let Some(v) = line.strip_prefix("read_bytes:") {
            read = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("write_bytes:") {
            write = v.trim().parse().ok();
        }
    }
    Some((read?, write?))
}

impl ProcScanner {
    pub fn new(proc_root: PathBuf, passwd: PathBuf) -> ProcScanner {
        ProcScanner {
            proc_root,
            passwd,
            page_kib: (procfs::page_size() / 1024).max(1),
            prev: HashMap::new(),
            names: HashMap::new(),
            users: HashMap::new(),
            users_loaded: false,
            prev_io: HashMap::new(),
            io_at: None,
            prev_total: None,
        }
    }

    /// Re-point `/proc` keeping every counter — how the fixture tests feed two
    /// recorded ticks through one scanner.
    pub fn set_proc_root(&mut self, root: PathBuf) {
        self.proc_root = root;
    }

    /// Forget the per-pid counters and the period: the next pass is a first
    /// pass (no percentages), not an average over however long we were away.
    pub fn forget_deltas(&mut self) {
        self.prev.clear();
        self.prev_total = None;
    }

    /// One pass. `total_ticks` is the aggregate `cpu` line's total at this
    /// instant (the same number `shares` uses), `active_cpus` the online count.
    pub fn scan(
        &mut self,
        total_ticks: u64,
        active_cpus: usize,
        mem_total_kib: u64,
        pid_digits: u8,
        columns: bool,
    ) -> Scan {
        let t0 = Instant::now();
        // The I/O rates are per *measured* interval between two Columns
        // passes, not per configured cadence.
        let secs = columns
            .then(|| {
                self.io_at
                    .map(|t| t0.saturating_duration_since(t).as_secs_f64())
            })
            .flatten();
        let mut next_io: HashMap<Ident, (u64, u64)> = HashMap::new();
        if columns {
            self.io_at = Some(t0);
        }
        self.load_users();
        let cpus = active_cpus.max(1);
        // htop: `period = Δtotal / activeCPUs`; the first pass has no period
        // and therefore no percentages — 0.0, never a fabricated burst.
        let period = self
            .prev_total
            .map(|p| total_ticks.saturating_sub(p) as f64 / cpus as f64)
            .filter(|p| *p > 0.0);
        self.prev_total = Some(total_ticks);
        let max_pct = (cpus as f32) * 100.0;

        let mut rows = Vec::with_capacity(self.prev.len().max(64));
        let mut next_prev = HashMap::with_capacity(self.prev.len().max(64));
        let mut kernel_threads = 0u64;
        let Ok(entries) = std::fs::read_dir(&self.proc_root) else {
            return Scan {
                table: ProcTable { rows, pid_digits },
                kernel_threads,
                ms: t0.elapsed().as_secs_f64() * 1000.0,
            };
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|n| n.parse::<i32>().ok())
            else {
                continue;
            };
            let dir = entry.path();
            // A pid can vanish between readdir and here: every read is optional.
            let Ok(stat_text) = std::fs::read(dir.join("stat")) else {
                continue;
            };
            let Ok(stat) = Stat::from_read(stat_text.as_slice()) else {
                continue;
            };
            let statm = std::fs::read(dir.join("statm"))
                .ok()
                .and_then(|b| StatM::from_read(b.as_slice()).ok());
            let uid = std::fs::metadata(&dir).map(|m| m.uid()).unwrap_or(0);
            let ident: Ident = (pid, stat.starttime);
            let ticks = stat.utime + stat.stime;
            let cpu_pct = match (self.prev.get(&ident), period) {
                (Some(prev), Some(period)) => {
                    ((ticks.saturating_sub(*prev) as f64 / period * 100.0) as f32).min(max_pct)
                }
                _ => 0.0,
            };
            next_prev.insert(ident, ticks);
            let (cmdline, comm) = self.name_of(ident, &dir, &stat.comm);
            // htop's I/O screen (arc 8a): `/proc/<pid>/io` is 0400 to its
            // owner, so this reads what it may and says which rows it
            // could. Only at `Detail::Columns` — it is one more open and
            // read per process (P15).
            let (read_bps, write_bps, io_readable) = if columns {
                match read_io(&dir) {
                    Some((rd, wr)) => {
                        let prev = self.prev_io.get(&ident).copied();
                        next_io.insert(ident, (rd, wr));
                        match (prev, secs) {
                            (Some((prd, pwr)), Some(dt)) if dt > 0.0 => (
                                (rd.saturating_sub(prd) as f64 / dt) as f32,
                                (wr.saturating_sub(pwr) as f64 / dt) as f32,
                                true,
                            ),
                            _ => (0.0, 0.0, true),
                        }
                    }
                    None => (0.0, 0.0, false),
                }
            } else {
                (0.0, 0.0, false)
            };
            let kthread = stat.flags & PF_KTHREAD != 0;
            if kthread {
                kernel_threads += 1;
            }
            let res_kib = stat.rss.saturating_mul(self.page_kib);
            rows.push(ProcRow {
                pid,
                ppid: stat.ppid,
                tgid: pid,
                uid,
                user: self.user_of(uid),
                state: stat.state,
                pri: stat
                    .priority
                    .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
                nice: stat.nice.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
                nlwp: stat.num_threads.max(0) as u32,
                virt_kib: stat.vsize / 1024,
                res_kib,
                shr_kib: statm
                    .map(|m| m.shared.saturating_mul(self.page_kib))
                    .unwrap_or(0),
                cpu_pct,
                mem_pct: if mem_total_kib > 0 {
                    (res_kib as f64 / mem_total_kib as f64 * 100.0) as f32
                } else {
                    0.0
                },
                time_cs: ticks_to_cs(ticks),
                starttime: stat.starttime,
                kthread,
                cmdline,
                comm,
                read_bps,
                write_bps,
                io_readable,
            });
        }
        // Forget what vanished, so a reused pid starts from nothing.
        if columns {
            self.prev_io = next_io;
        }
        self.prev = next_prev;
        self.names.retain(|k, _| self.prev.contains_key(k));
        Scan {
            table: ProcTable { rows, pid_digits },
            kernel_threads,
            ms: t0.elapsed().as_secs_f64() * 1000.0,
        }
    }

    /// `cmdline` on first sight, `comm` for a kernel thread or a zombie whose
    /// cmdline is empty (htop prints the comm, without ps's brackets).
    fn name_of(&mut self, ident: Ident, dir: &Path, comm: &str) -> (Arc<str>, Arc<str>) {
        if let Some(n) = self.names.get(&ident) {
            return n.clone();
        }
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        let joined: String = cmdline
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let comm: Arc<str> = Arc::from(comm);
        let cmdline: Arc<str> = if joined.is_empty() {
            comm.clone()
        } else {
            Arc::from(joined.as_str())
        };
        self.names.insert(ident, (cmdline.clone(), comm.clone()));
        (cmdline, comm)
    }

    fn user_of(&mut self, uid: u32) -> Arc<str> {
        self.users
            .entry(uid)
            .or_insert_with(|| Arc::from(uid.to_string().as_str()))
            .clone()
    }

    /// `passwd` once: `name:x:uid:…`. Re-read never (htop caches too).
    fn load_users(&mut self) {
        if self.users_loaded {
            return;
        }
        self.users_loaded = true;
        let Ok(text) = std::fs::read_to_string(&self.passwd) else {
            return;
        };
        for line in text.lines() {
            let mut f = line.split(':');
            let (Some(name), _, Some(uid)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            if let Ok(uid) = uid.parse::<u32>() {
                self.users.insert(uid, Arc::from(name));
            }
        }
    }
}

/// Jiffies → centiseconds. `CLK_TCK` is 100 on every Linux we care about, so
/// this is the identity there; the general form keeps a 250 Hz kernel honest.
fn ticks_to_cs(ticks: u64) -> u64 {
    let hz = procfs::ticks_per_second();
    if hz == 100 {
        ticks
    } else {
        (ticks as u128 * 100 / u128::from(hz.max(1))) as u64
    }
}
