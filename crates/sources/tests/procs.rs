//! The pid-level scan (§8.1, P15, brief 2a task 2) against `fixtures/procfs/`
//! — two recorded ticks of torch's `/proc`, 2 s apart — and, `#[ignore]`d,
//! the live pass timed on this machine.

use std::path::{Path, PathBuf};

use gridwatch_sources::cpu::{CpuSampler, ProcScanner, Roots, parse_stat};
use gridwatch_store::keys::{cpu, sys};
use gridwatch_store::{Datum, Detail, Sampler, Ts};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/procfs")
}

fn tick(n: u8) -> PathBuf {
    fixtures().join(format!("tick{n}"))
}

fn total_ticks(proc_root: &Path) -> u64 {
    let text = std::fs::read_to_string(proc_root.join("stat")).unwrap();
    parse_stat(&text).unwrap().0.total()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            // The fixtures are committed read-only; the copy must be editable.
            std::fs::copy(e.path(), &dest).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}

fn passwd(dir: &Path) -> PathBuf {
    let p = dir.join("passwd");
    // The fixture directories are owned by whoever checked the repo out, so
    // the scan's uid is ours: map it to a name we can assert on.
    let uid = std::fs::metadata(tick(1).join("1")).map(|m| {
        use std::os::unix::fs::MetadataExt;
        m.uid()
    });
    let uid = uid.unwrap_or(0);
    std::fs::write(
        &p,
        format!("root:x:0:0:root:/root:/bin/sh\ntester:x:{uid}:{uid}::/home/t:/bin/sh\n"),
    )
    .unwrap();
    p
}

/// Irix-mode CPU% keyed by `(pid, starttime)` over `period = Δtotal /
/// active_cpus`, htop's `LinuxProcessTable.c`: pid 920164 burned a measured
/// number of jiffies between the two recorded ticks.
#[test]
fn cpu_percent_is_irix_mode_over_the_aggregate_period() {
    let dir = std::env::temp_dir().join(format!("gw-procs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pw = passwd(&dir);
    let mut sc = ProcScanner::new(tick(1), pw.clone());
    let first = sc.scan(total_ticks(&tick(1)), 32, 91 * 1024 * 1024, 7, false, false);
    assert!(first.ms >= 0.0);
    let row = first
        .table
        .rows
        .iter()
        .find(|r| r.pid == 920_164)
        .expect("pid 920164 in tick1");
    assert_eq!(
        row.cpu_pct, 0.0,
        "no period on the first pass, so no percentage"
    );
    assert_eq!(&*row.user, "tester", "uid resolved through the passwd file");
    assert!(!row.kthread);
    assert_eq!(row.tgid, row.pid);
    assert!(row.res_kib > 0 && row.virt_kib >= row.res_kib);
    assert_eq!(row.time_cs, 0, "the recorded bash had burned no ticks yet");
    let init = first.table.rows.iter().find(|r| r.pid == 1).unwrap();
    assert_eq!(
        &*init.cmdline, &*init.comm,
        "no cmdline file in the fixture → comm"
    );
    let kthreadd = first.table.rows.iter().find(|r| r.pid == 2).unwrap();
    assert!(kthreadd.kthread, "PF_KTHREAD from stat field 9");
    assert_eq!(first.kernel_threads, 1);
    assert_eq!(first.table.pid_digits, 7);

    // Second tick through the same scanner: the sampler re-points its root
    // and keeps every counter, exactly as the live source keeps them.
    let expected = {
        // htop's arithmetic by hand from the two stat files.
        let read = |n: u8| -> (u64, u64) {
            let text = std::fs::read_to_string(tick(n).join("920164/stat")).unwrap();
            let after = text.rsplit(')').next().unwrap();
            let f: Vec<&str> = after.split_whitespace().collect();
            // fields after comm: state(0) ppid(1) … utime(11) stime(12)
            (f[11].parse().unwrap(), f[12].parse().unwrap())
        };
        let (u1, s1) = read(1);
        let (u2, s2) = read(2);
        let period = (total_ticks(&tick(2)) - total_ticks(&tick(1))) as f64 / 32.0;
        ((u2 + s2 - u1 - s1) as f64 / period * 100.0) as f32
    };
    let mut sampler = CpuSampler::new(Roots {
        proc: tick(1),
        sys: dir.join("nosys"),
        passwd: dir.join("passwd"),
    });
    let a = sampler.sample(Ts(0), Detail::Table).unwrap();
    assert!(
        a.iter().any(|s| s.id.name == "proc.table"),
        "the table is published at Detail::Table"
    );
    let sampler = sampler.with_proc_root(tick(2));
    // A third tick synthesised from tick2: the process gains 150 jiffies while
    // the machine gains 200 per CPU (6400 on the aggregate line), so htop's
    // Irix-mode answer is 150 / 200 × 100 = 75.0 %.
    let tick3 = dir.join("tick3");
    copy_dir(&tick(2), &tick3);
    let stat = std::fs::read_to_string(tick3.join("920164/stat")).unwrap();
    let (head, tail) = stat.split_once(") ").unwrap();
    let mut f: Vec<String> = tail.split_whitespace().map(String::from).collect();
    f[11] = "150".into();
    std::fs::write(
        tick3.join("920164/stat"),
        format!("{head}) {}\n", f.join(" ")),
    )
    .unwrap();
    let agg = std::fs::read_to_string(tick3.join("stat")).unwrap();
    let mut lines: Vec<String> = agg.lines().map(String::from).collect();
    let mut cpu: Vec<String> = lines[0].split_whitespace().map(String::from).collect();
    cpu[1] = (cpu[1].parse::<u64>().unwrap() + 6400).to_string();
    lines[0] = cpu.join(" ");
    std::fs::write(tick3.join("stat"), lines.join("\n") + "\n").unwrap();
    let expected3 = 75.0f32;
    let mut sampler = sampler;
    let b = sampler.sample(Ts(2_000_000_000), Detail::Table).unwrap();
    let table = b
        .iter()
        .find_map(|s| match (&s.id.name, &s.datum) {
            (n, Datum::Record(r)) if *n == "proc.table" => {
                r.as_any().downcast_ref::<cpu::ProcTable>().cloned()
            }
            _ => None,
        })
        .expect("proc.table on the second tick");
    let row = table.rows.iter().find(|r| r.pid == 920_164).unwrap();
    assert!(
        (row.cpu_pct - expected).abs() < 0.01,
        "cpu% {} vs htop's {expected}",
        row.cpu_pct
    );
    assert_eq!(row.time_cs, 0);
    let mut sampler = sampler.with_proc_root(tick3.clone());
    let c = sampler.sample(Ts(4_000_000_000), Detail::Table).unwrap();
    let table3 = c
        .iter()
        .find_map(|s| match (&s.id.name, &s.datum) {
            (n, Datum::Record(r)) if *n == "proc.table" => {
                r.as_any().downcast_ref::<cpu::ProcTable>().cloned()
            }
            _ => None,
        })
        .unwrap();
    let row3 = table3.rows.iter().find(|r| r.pid == 920_164).unwrap();
    assert!(
        (row3.cpu_pct - expected3).abs() < 0.01,
        "cpu% {} vs htop's {expected3}",
        row3.cpu_pct
    );
    assert_eq!(row3.time_cs, 150, "TIME+ is utime + stime in centiseconds");
    let kernel = b
        .iter()
        .find(|s| s.id.name == sys::TASKS_KERNEL.id.name)
        .expect("tasks.kernel is produced with the scan");
    assert!(matches!(kernel.datum, Datum::Scalar(v) if v == 1.0));
    assert!(
        b.iter().any(|s| s.id.name == sys::SCAN_MS.id.name),
        "sys.scan_ms carries the pass time"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Below `Detail::Table` the scan does not run and none of its keys appear.
#[test]
fn meters_detail_never_scans() {
    let dir = std::env::temp_dir().join(format!("gw-procs-m-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut sampler = CpuSampler::new(Roots {
        proc: tick(1),
        sys: dir.join("nosys"),
        passwd: passwd(&dir),
    });
    let a = sampler.sample(Ts(0), Detail::Meters).unwrap();
    for s in &a {
        assert!(
            !["proc.table", "tasks.kernel", "sys.scan_ms"].contains(&s.id.name),
            "{} published at Detail::Meters",
            s.id.name
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// P15 on this machine: the pass ≤ 20 ms wall. Timing, so ignored by default:
/// `cargo test -p gridwatch-sources --release --test procs -- --ignored`.
#[test]
#[ignore = "timing; run in release on torch"]
fn live_scan_is_inside_p15() {
    let mut sc = ProcScanner::new(PathBuf::from("/proc"), PathBuf::from("/etc/passwd"));
    let total = total_ticks(Path::new("/proc"));
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let _ = sc.scan(total, cpus, 91 * 1024 * 1024, 7, false, false);
    let mut worst = 0.0f64;
    let mut sum = 0.0f64;
    let n = 10;
    for _ in 0..n {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let total = total_ticks(Path::new("/proc"));
        let s = sc.scan(total, cpus, 91 * 1024 * 1024, 7, false, false);
        worst = worst.max(s.ms);
        sum += s.ms;
        println!(
            "scan: {} rows, {} kthreads, {:.2} ms",
            s.table.rows.len(),
            s.kernel_threads,
            s.ms
        );
    }
    println!(
        "P15: mean {:.2} ms, worst {worst:.2} ms over {n} passes",
        sum / f64::from(n)
    );
    assert!(worst <= 20.0, "P15: worst pass {worst:.2} ms > 20 ms");
}

/// Arc 8a: the gated `/proc/<pid>/io` file is read only at
/// `Detail::Columns`, and its rates come from the interval between two
/// such passes. This scans our own tree, where at least our own process is
/// readable.
#[test]
fn the_io_columns_are_read_only_when_asked_for() {
    let mut sc = ProcScanner::new(PathBuf::from("/proc"), PathBuf::from("/etc/passwd"));
    let total = total_ticks(Path::new("/proc"));
    // Without the flag: nothing is read, and every row says so.
    let plain = sc.scan(total, 32, 91 * 1024 * 1024, 7, false, false);
    assert!(!plain.table.rows.is_empty());
    assert!(
        plain.table.rows.iter().all(|r| !r.io_readable),
        "no io file is opened at Detail::Table"
    );
    assert!(
        plain
            .table
            .rows
            .iter()
            .all(|r| r.read_bps == 0.0 && r.write_bps == 0.0)
    );
    // With it: our own rows are readable, and another user's are not — the
    // scan says which rather than showing zeroes as if they were idle.
    let first = sc.scan(total, 32, 91 * 1024 * 1024, 7, true, false);
    let me = std::process::id() as i32;
    let ours = first
        .table
        .rows
        .iter()
        .find(|r| r.pid == me)
        .expect("our own row");
    assert!(ours.io_readable, "our own /proc/self/io is readable");
    // The first pass has no interval, so it reports no rate rather than a
    // fabricated burst — the same rule the CPU% column follows.
    assert_eq!(ours.read_bps, 0.0);
    assert_eq!(ours.write_bps, 0.0);
    // A second pass has one, and the rate is finite and not negative.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let _ = std::fs::read_to_string("/proc/self/stat");
    let second = sc.scan(total, 32, 91 * 1024 * 1024, 7, true, false);
    let ours = second
        .table
        .rows
        .iter()
        .find(|r| r.pid == me)
        .expect("our own row");
    assert!(
        ours.read_bps >= 0.0 && ours.read_bps.is_finite(),
        "{ours:?}"
    );
    assert!(ours.write_bps >= 0.0 && ours.write_bps.is_finite());
}

/// P15 with the gated files on (arc 8a): the same pass, plus one
/// `/proc/<pid>/io` open and read per process. This is what htop's `H` and
/// its I/O screen cost, and the ceiling is the reason they are behind
/// `Detail::Columns` rather than on by default.
///
/// `cargo test -p gridwatch-sources --release --test procs -- --ignored`.
#[test]
#[ignore = "timing; run in release on torch"]
fn live_scan_with_the_gated_files_is_inside_p15() {
    let mut sc = ProcScanner::new(PathBuf::from("/proc"), PathBuf::from("/etc/passwd"));
    let total = total_ticks(Path::new("/proc"));
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let _ = sc.scan(total, cpus, 91 * 1024 * 1024, 7, true, false);
    let mut worst = 0.0f64;
    let mut sum = 0.0f64;
    let n = 10;
    let mut readable = 0;
    for _ in 0..n {
        let s = sc.scan(total, cpus, 91 * 1024 * 1024, 7, true, false);
        readable = s.table.rows.iter().filter(|r| r.io_readable).count();
        println!(
            "columns scan: {} rows, {readable} readable io, {:.2} ms",
            s.table.rows.len(),
            s.ms
        );
        worst = worst.max(s.ms);
        sum += s.ms;
    }
    println!("mean {:.2} ms, worst {worst:.2} ms", sum / n as f64);
    assert!(readable > 0, "at least our own processes are readable");
    // P15's ceiling is 12 ms; the pid-level pass measures ~6 ms, so the
    // gated read has ~6 ms of room and this fails if it eats it.
    assert!(worst < 12.0, "the gated pass took {worst:.2} ms");
}

/// Arc 10b (D60) — htop's `H`: the `task/` walk arc 8a's demand path asked
/// for and nobody wrote, so the toggle changed what was asked for and not
/// what was shown. Live, because the fixtures have no `task/` directories and
/// a fixture that pretended to would prove nothing about the real layout.
#[test]
fn the_task_walk_lists_threads_under_their_leader() {
    let mut sc = ProcScanner::new(PathBuf::from("/proc"), PathBuf::from("/etc/passwd"));
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let total = || total_ticks(Path::new("/proc"));
    sc.scan(total(), cpus, 91 * 1024 * 1024, 7, false, false);
    let without = sc.scan(total(), cpus, 91 * 1024 * 1024, 7, true, false);
    let with = sc.scan(total(), cpus, 91 * 1024 * 1024, 7, true, true);

    // The I/O screen raises the same `Detail::Columns` and must not pay for
    // a walk it has no use for.
    assert!(
        without.table.rows.iter().all(|r| r.tgid == r.pid),
        "the walk must not run unless asked"
    );
    let leaders: std::collections::HashSet<i32> =
        without.table.rows.iter().map(|r| r.pid).collect();
    let threads: Vec<_> = with.table.rows.iter().filter(|r| r.tgid != r.pid).collect();
    assert!(
        threads.len() > 10,
        "this box runs thousands of threads; the walk found {}",
        threads.len()
    );
    for t in &threads {
        assert_ne!(t.pid, t.tgid, "a thread row is never its own leader");
        assert!(!t.comm.is_empty(), "a thread row carries its own name");
        assert_eq!(
            t.nlwp, 1,
            "a thread is one LWP; the count belongs to the group"
        );
    }
    // A thread's leader is a real row in the same table — allowing for the
    // ones that exited between the two passes.
    let orphans = threads
        .iter()
        .filter(|t| !leaders.contains(&t.tgid))
        .count();
    assert!(
        orphans * 20 < threads.len().max(20),
        "{orphans} of {} threads claim a leader that is not in the table",
        threads.len()
    );
    println!(
        "task walk: {} leaders, {} thread rows",
        leaders.len(),
        threads.len()
    );
}

/// P15's `task/` row: the walk is budgeted at **+30 ms** on top of the
/// pid-level pass, and only ever runs at `Detail::Columns` with `H` on.
/// Timing, so ignored by default like the pass it sits on:
/// `cargo test -p gridwatch-sources --release --test procs -- --ignored`.
#[test]
#[ignore = "timing; run in release on torch"]
fn the_task_walk_is_inside_p15() {
    let mut sc = ProcScanner::new(PathBuf::from("/proc"), PathBuf::from("/etc/passwd"));
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let total = || total_ticks(Path::new("/proc"));
    sc.scan(total(), cpus, 91 * 1024 * 1024, 7, true, true);
    let (mut plain, mut walked, mut rows, mut threads) = (0.0f64, 0.0f64, 0usize, 0usize);
    let n = 10;
    for _ in 0..n {
        std::thread::sleep(std::time::Duration::from_millis(300));
        plain += sc.scan(total(), cpus, 91 * 1024 * 1024, 7, true, false).ms;
        std::thread::sleep(std::time::Duration::from_millis(300));
        let s = sc.scan(total(), cpus, 91 * 1024 * 1024, 7, true, true);
        walked += s.ms;
        rows = s.table.rows.len();
        threads = s.table.rows.iter().filter(|r| r.tgid != r.pid).count();
    }
    let (plain, walked) = (plain / f64::from(n), walked / f64::from(n));
    println!(
        "P15 task walk: {plain:.1} ms without, {walked:.1} ms with ({threads} thread rows of \
         {rows}), +{:.1} ms",
        walked - plain
    );
    assert!(
        walked - plain <= 30.0,
        "P15 budgets +30 ms for the task walk; it cost +{:.1} ms",
        walked - plain
    );
}
