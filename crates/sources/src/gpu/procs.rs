//! The GPU process rows (§8.1): the two v3 lists merged by PID (`Both` when a
//! PID is in each — nvtop prints it twice), overlaid with the utilisation
//! samples newer than `last_seen` (nvtop's carry-forward timestamp), the
//! source's own pid filtered out (P12). Pure functions, tested without NVML.

use gridwatch_store::keys::gpu::{GpuProcKind, GpuProcRow, GpuProcs};

use super::probe::{ProcMem, ProcUtil};

/// Merge the graphics and compute lists. Rows come out in ascending PID order
/// so a journal line is stable across ticks; `own_pid` never appears.
pub fn merge(graphics: &[ProcMem], compute: &[ProcMem], own_pid: u32) -> Vec<GpuProcRow> {
    let mut rows: Vec<GpuProcRow> = Vec::with_capacity(graphics.len() + compute.len());
    for (list, kind) in [
        (graphics, GpuProcKind::Graphics),
        (compute, GpuProcKind::Compute),
    ] {
        for p in list {
            if p.pid == own_pid {
                continue;
            }
            if let Some(existing) = rows.iter_mut().find(|r| r.pid as u32 == p.pid) {
                if existing.kind != kind {
                    existing.kind = GpuProcKind::Both;
                }
                // Both lists report the same context memory; keep the larger
                // in case one side says `Unavailable`.
                existing.vram_b = match (existing.vram_b, p.vram_b) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                continue;
            }
            rows.push(GpuProcRow {
                pid: p.pid as i32,
                kind,
                vram_b: p.vram_b,
                sm_pct: 0,
                mem_pct: 0,
                enc_pct: 0,
                dec_pct: 0,
                fresh: false,
            });
        }
    }
    rows.sort_by_key(|r| r.pid);
    rows
}

/// Overlay utilisation samples: only samples newer than `last_seen` count, a
/// value above 100 is discarded (nvtop), a PID without a fresh sample keeps
/// zeros with `fresh = false`. Returns the newest timestamp seen, which the
/// caller carries forward as the next `last_seen`.
pub fn overlay(rows: &mut [GpuProcRow], samples: &[ProcUtil], last_seen: u64) -> u64 {
    let mut newest = last_seen;
    for s in samples {
        if s.timestamp_us <= last_seen {
            continue;
        }
        newest = newest.max(s.timestamp_us);
        if s.sm > 100 || s.mem > 100 || s.enc > 100 || s.dec > 100 {
            continue;
        }
        if let Some(r) = rows.iter_mut().find(|r| r.pid as u32 == s.pid) {
            r.sm_pct = s.sm;
            r.mem_pct = s.mem;
            r.enc_pct = s.enc;
            r.dec_pct = s.dec;
            r.fresh = true;
        }
    }
    newest
}

pub fn table(rows: Vec<GpuProcRow>, vram_total_b: u64) -> GpuProcs {
    GpuProcs { rows, vram_total_b }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pm(pid: u32, mib: Option<u64>) -> ProcMem {
        ProcMem {
            pid,
            vram_b: mib.map(|m| m * 1024 * 1024),
        }
    }

    #[test]
    fn both_lists_merge_by_pid_and_own_pid_is_dropped() {
        let g = [
            pm(1701, Some(464)),
            pm(412345, Some(12579)),
            pm(999, Some(1)),
        ];
        let c = [pm(11805, Some(44)), pm(412345, None)];
        let rows = merge(&g, &c, 999);
        let pids: Vec<i32> = rows.iter().map(|r| r.pid).collect();
        assert_eq!(pids, vec![1701, 11805, 412345]);
        let game = rows.iter().find(|r| r.pid == 412345).unwrap();
        assert_eq!(game.kind, GpuProcKind::Both);
        assert_eq!(
            game.vram_b,
            Some(12579 * 1024 * 1024),
            "Unavailable never wins"
        );
        assert_eq!(rows[0].kind, GpuProcKind::Graphics);
        assert_eq!(rows[1].kind, GpuProcKind::Compute);
        assert!(rows.iter().all(|r| !r.fresh && r.sm_pct == 0));
    }

    #[test]
    fn overlay_honours_last_seen_and_caps_at_100() {
        let mut rows = merge(&[pm(1, Some(1)), pm(2, Some(2))], &[], 0);
        let samples = [
            ProcUtil {
                pid: 1,
                timestamp_us: 1_000,
                sm: 17,
                mem: 9,
                enc: 0,
                dec: 0,
            },
            ProcUtil {
                pid: 2,
                timestamp_us: 500, // stale: not newer than last_seen
                sm: 50,
                mem: 0,
                enc: 0,
                dec: 0,
            },
            ProcUtil {
                pid: 2,
                timestamp_us: 2_000,
                sm: 300, // garbage: discarded, as nvtop does
                mem: 0,
                enc: 0,
                dec: 0,
            },
        ];
        let newest = overlay(&mut rows, &samples, 500);
        assert_eq!(newest, 2_000, "the newest timestamp is carried forward");
        assert_eq!(
            (rows[0].sm_pct, rows[0].mem_pct, rows[0].fresh),
            (17, 9, true)
        );
        assert_eq!((rows[1].sm_pct, rows[1].fresh), (0, false));
    }
}
