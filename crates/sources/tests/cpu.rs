//! htop-formula tests against `fixtures/procfs/` (§12.1, brief 1b task 6).
//! The fixtures are two ticks of torch's real `/proc`, recorded 2 s apart, so
//! the deltas the sampler computes are the ones the machine produced.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gridwatch_sources::cpu::sysfs;
use gridwatch_sources::cpu::{CpuSampler, Roots, Ticks, parse_stat, shares};
use gridwatch_store::keys::cpu::CpuTopology;
use gridwatch_store::{Datum, Detail, Sampler, Ts, demo};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/procfs")
}

fn tick(n: u8) -> PathBuf {
    fixtures().join(format!("tick{n}"))
}

/// torch's sysfs, materialised: die string `00000000111111110000000011111111`
/// (MACHINE.md / research digest §5), SMT sibling of cpu N is cpu N+16, and
/// k10temp exposes temp1/temp3/temp4 — **there is no temp2**.
fn write_sysfs(root: &Path) {
    const DIES: &str = "00000000111111110000000011111111";
    for (cpu, die) in DIES.bytes().enumerate() {
        let topo = root.join(format!("devices/system/cpu/cpu{cpu}/topology"));
        std::fs::create_dir_all(&topo).unwrap();
        std::fs::write(topo.join("die_id"), format!("{}\n", die - b'0')).unwrap();
        std::fs::write(topo.join("core_id"), format!("{}\n", cpu % 16)).unwrap();
        let freq = root.join(format!("devices/system/cpu/cpu{cpu}/cpufreq"));
        std::fs::create_dir_all(&freq).unwrap();
        let khz = if cpu % 16 < 8 { 4_900_123 } else { 5_545_839 };
        std::fs::write(freq.join("scaling_cur_freq"), format!("{khz}\n")).unwrap();
    }
    let hwmon = root.join("class/hwmon/hwmon4");
    std::fs::create_dir_all(&hwmon).unwrap();
    std::fs::write(hwmon.join("name"), "k10temp\n").unwrap();
    for (idx, label, milli) in [
        (1, "Tctl", 63_000),
        (3, "Tccd1", 61_500),
        (4, "Tccd2", 54_125),
    ] {
        std::fs::write(hwmon.join(format!("temp{idx}_label")), format!("{label}\n")).unwrap();
        std::fs::write(hwmon.join(format!("temp{idx}_input")), format!("{milli}\n")).unwrap();
    }
    // A neighbouring chip that must never be picked up.
    let other = root.join("class/hwmon/hwmon0");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("name"), "nvme\n").unwrap();
    std::fs::write(other.join("temp1_label"), "Composite\n").unwrap();
    std::fs::write(other.join("temp1_input"), "41000\n").unwrap();
}

struct TempTree(PathBuf);

impl TempTree {
    fn new(tag: &str) -> TempTree {
        let p = std::env::temp_dir().join(format!(
            "gridwatch-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempTree(p)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(samples: &[gridwatch_store::Sample]) -> BTreeSet<&'static str> {
    samples.iter().map(|s| s.id.name).collect()
}

/// The `/proc/stat` breakdown: guest subtraction, systemall, virt and the
/// saturating deltas, checked against the recorded ticks by hand.
#[test]
fn stat_breakdown_follows_htop() {
    // cpu0, tick1 → tick2 (recorded): user 2319430 → 2319440, nice 987 → 987,
    // system 728811 → 728813, idle 13743474 → 13743662, iowait 2557, irq 0,
    // softirq 76762, steal/guest/guest_nice 0.
    let a = Ticks {
        user: 2_319_430,
        nice: 987,
        systemall: 728_811 + 76_762,
        idle: 13_743_474,
        iowait: 2_557,
        steal: 0,
        virt: 0,
    };
    let b = Ticks {
        user: 2_319_440,
        nice: 987,
        systemall: 728_813 + 76_762,
        idle: 13_743_662,
        iowait: 2_557,
        steal: 0,
        virt: 0,
    };
    let s = shares(a, b);
    // Δuser 10 + Δnice 0 + Δsystemall 2 + Δidleall 188.
    let period = 200.0f32;
    assert!((s.user - 10.0 / period).abs() < 1e-6);
    assert!((s.kernel - 2.0 / period).abs() < 1e-6);
    assert_eq!(s.nice, 0.0);
    assert_eq!(s.virt, 0.0);
    assert_eq!(s.iowait, 0.0);
    // busy = 1 − idle share, exactly (iowait counts as idle).
    assert!((s.busy() - 12.0 / period).abs() < 1e-6);
}

#[test]
fn guest_is_subtracted_from_user_and_nice() {
    // A /proc/stat line with guest time counted inside user/nice, as the kernel
    // reports it: htop subtracts guest before anything else.
    //        user nice system idle iowait irq softirq steal guest guest_nice
    let text = concat!(
        "cpu  100 20 10 500 5 1 2 3 40 7\n",
        "cpu0 100 20 10 500 5 1 2 3 40 7\n",
        "ctxt 1\nbtime 1\nprocesses 1\nprocs_running 2\nprocs_blocked 0\n"
    );
    let (ticks, cores, _) = parse_stat(text).expect("parses");
    assert_eq!(cores.len(), 1);
    assert_eq!(ticks.user, 60, "user must lose guest");
    assert_eq!(ticks.nice, 13, "nice must lose guest_nice");
    assert_eq!(ticks.systemall, 13, "system + irq + softirq");
    assert_eq!(ticks.virt, 47, "guest + guest_nice");
    assert_eq!(ticks.idle + ticks.iowait, 505, "idleall");
    assert_eq!(ticks.total(), 60 + 13 + 13 + 505 + 3 + 47);
}

#[test]
fn deltas_never_underflow_on_a_counter_reset() {
    let big = Ticks {
        user: 10,
        nice: 10,
        systemall: 10,
        idle: 10,
        iowait: 0,
        steal: 0,
        virt: 0,
    };
    let small = Ticks::default();
    // A rolled-back counter (a reset, a container's remounted /proc) must give
    // zeroes, never a wrapped enormous share.
    let s = shares(big, small);
    assert!(s.busy() >= 0.0 && s.busy() <= 1.0, "{s:?}");
}

#[test]
fn first_tick_has_no_percentages_and_the_second_has_all_of_them() {
    let mut s = CpuSampler::new(Roots {
        proc: tick(1),
        sys: PathBuf::from("/nonexistent-sysfs"),
    });
    let first = s.sample(Ts(0), Detail::Meters).expect("tick 1 samples");
    let n = names(&first);
    assert!(
        !n.contains("cpu.total_pct") && !n.contains("cpu.core_pct"),
        "the first scan has no delta — it must not fabricate a 0: {n:?}"
    );
    assert!(n.contains("mem.total_b"), "memory needs no delta");
    assert!(n.contains("sys.load1"));

    // Feed tick1 then tick2 through one sampler: it keeps the previous ticks,
    // not the path, so re-pointing the root replays the recorded second scan.
    let mut s = CpuSampler::new(Roots {
        proc: tick(1),
        sys: PathBuf::from("/nonexistent-sysfs"),
    });
    let _ = s.sample(Ts(0), Detail::Meters).unwrap();
    let mut s = s.with_proc_root(tick(2));
    let second = s.sample(Ts(1), Detail::Meters).expect("tick 2 samples");
    let n = names(&second);
    assert!(n.contains("cpu.total_pct"));
    assert!(n.contains("cpu.core_pct"));
    assert!(n.contains("cpu.breakdown"));
    let cores = second
        .iter()
        .filter(|s| s.id.name == "cpu.core_pct")
        .count();
    assert_eq!(cores, 32, "torch has 32 logical CPUs in the fixture");
    for s in second.iter().filter(|s| s.id.name == "cpu.core_pct") {
        let Datum::Scalar(v) = s.datum else {
            panic!("core_pct must be scalar")
        };
        assert!((0.0..=100.0).contains(&v), "core {:?} = {v}", s.id.label);
    }
}

#[test]
fn memory_formulas_match_htop() {
    let mut s = CpuSampler::new(Roots {
        proc: tick(1),
        sys: PathBuf::from("/nonexistent-sysfs"),
    });
    let out = s.sample(Ts(0), Detail::Meters).unwrap();
    let val = |name: &str| -> f64 {
        out.iter()
            .find(|s| s.id.name == name)
            .and_then(|s| match s.datum {
                Datum::Scalar(v) => Some(v),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    // Recompute htop's formulas straight from the recorded text.
    let text = std::fs::read_to_string(tick(1).join("meminfo")).unwrap();
    let kib = |key: &str| -> f64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
            * 1024.0
    };
    let total = kib("MemTotal:");
    let free = kib("MemFree:");
    let cached_raw = kib("Cached:");
    let sreclaim = kib("SReclaimable:");
    let shmem = kib("Shmem:");
    let buffers = kib("Buffers:");
    assert_eq!(val("mem.total_b"), total);
    assert_eq!(val("mem.cached_b"), cached_raw + sreclaim - shmem);
    assert_eq!(
        val("mem.used_b"),
        total - (free + cached_raw + sreclaim + buffers)
    );
    assert_eq!(val("mem.shared_b"), shmem);
    assert_eq!(
        val("swap.used_b"),
        kib("SwapTotal:") - kib("SwapFree:") - kib("SwapCached:")
    );
    // The meter's segments can never overflow its bar.
    let segments =
        val("mem.used_b") + val("mem.shared_b") + val("mem.buffers_b") + val("mem.cached_b");
    assert!(segments <= total, "{segments} > {total}");
    assert!(
        (segments - (total - free)).abs() < 1.0,
        "segments = total − free"
    );
}

#[test]
fn pid_digits_and_process_count_come_from_the_fixture() {
    assert_eq!(sysfs::pid_digits(&tick(1)), 7, "pid_max 4194304 → 7 digits");
    assert_eq!(
        sysfs::process_count(&tick(1)),
        Some(3),
        "three pid directories are recorded"
    );
    // A pid_max no kernel would print still clamps into the column's range.
    assert_eq!(sysfs::pid_digits(Path::new("/nonexistent")), 5);
}

#[test]
fn topology_groups_torch_into_two_ccds_with_smt_pairs() {
    let tree = TempTree::new("sysfs-topo");
    write_sysfs(&tree.0);
    let topo = sysfs::topology(&tree.0, 32);
    let dies = topo.dies();
    assert_eq!(dies.len(), 2, "two CCDs");
    let (die0, cores0) = &dies[0];
    assert_eq!(*die0, 0);
    assert_eq!(cores0.len(), 8, "8 physical cores per CCD");
    assert_eq!(cores0[0], vec![0, 16], "SMT sibling of cpu0 is cpu16");
    assert_eq!(cores0[7], vec![7, 23]);
    let (die1, cores1) = &dies[1];
    assert_eq!(*die1, 1);
    assert_eq!(cores1[0], vec![8, 24]);
    assert_eq!(cores1[7], vec![15, 31]);
    // Every logical CPU appears exactly once.
    let all: BTreeSet<u16> = dies
        .iter()
        .flat_map(|(_, cores)| cores.iter().flatten().copied())
        .collect();
    assert_eq!(all.len(), 32);
}

#[test]
fn k10temp_is_resolved_by_label_not_by_index() {
    let tree = TempTree::new("sysfs-temp");
    write_sysfs(&tree.0);
    let inputs = sysfs::temp_inputs(&tree.0, "k10temp");
    let labels: Vec<&str> = inputs.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["k10temp:Tctl", "k10temp:Tccd1", "k10temp:Tccd2"],
        "temp2 does not exist on torch: indices are never assumed contiguous"
    );
    // temp3 is Tccd1 — reading by index would have reported the wrong die.
    let tccd1 = inputs.iter().find(|t| t.label.ends_with("Tccd1")).unwrap();
    assert!(tccd1.path.ends_with("temp3_input"));
    assert_eq!(sysfs::temp_c(&tccd1.path), Some(61.5));
    assert!(
        sysfs::temp_inputs(&tree.0, "k10temp")
            .iter()
            .all(|t| !t.label.contains("Composite")),
        "the nvme chip must not leak into the cpu source's keys"
    );
}

#[test]
fn die_temperature_labels_follow_the_die_map() {
    let tree = TempTree::new("sysfs-die");
    write_sysfs(&tree.0);
    let mut s = CpuSampler::new(Roots {
        proc: tick(1),
        sys: tree.0.clone(),
    });
    let _ = s.sample(Ts(0), Detail::Meters).unwrap();
    let topo: &CpuTopology = s.topology();
    assert_eq!(topo.temp_label(0), Some("k10temp:Tccd1"));
    assert_eq!(topo.temp_label(1), Some("k10temp:Tccd2"));
    assert_eq!(topo.temp_label(2), None);
}

/// §12.5: the synth exists so demo mode and the live source cannot drift. If
/// this fails, either the sampler stopped emitting a key or the synth invented
/// one — both make every demo-driven snapshot a lie about the live tile.
#[test]
fn demo_and_live_emit_the_same_key_names() {
    let tree = TempTree::new("sysfs-keys");
    write_sysfs(&tree.0);
    let mut s = CpuSampler::new(Roots {
        proc: tick(1),
        sys: tree.0.clone(),
    });
    // The union over both ticks: the first scan has no deltas and publishes the
    // topology, the second has the percentages — together they are every key
    // the live source can produce.
    let mut live = names(&s.sample(Ts(0), Detail::Meters).unwrap());
    let mut s = s.with_proc_root(tick(2));
    live.extend(names(&s.sample(Ts(1), Detail::Meters).unwrap()));

    let mut synth = demo::CpuSynth::new(7);
    let mut demo_names = names(&synth.tick(Ts(1_500_000_000)).samples);
    demo_names.extend(names(&synth.tick(Ts(3_000_000_000)).samples));

    let live_only: Vec<_> = live.difference(&demo_names).collect();
    let demo_only: Vec<_> = demo_names.difference(&live).collect();
    assert!(
        live_only.is_empty() && demo_only.is_empty(),
        "demo/live key drift — live only: {live_only:?}, demo only: {demo_only:?}"
    );
}

/// Hardware-gated (§12.6): runs against the real `/proc` and `/sys` on torch.
/// `cargo test -p gridwatch-sources -- --ignored live_scan`
#[test]
#[ignore = "reads the live /proc and /sys; run by hand on torch"]
fn live_scan_reports_plausible_numbers() {
    let mut s = CpuSampler::new(Roots::default());
    let _ = s.sample(Ts(0), Detail::Meters).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    let out = s.sample(Ts(1), Detail::Meters).unwrap();
    let scalars: Vec<(String, f64)> = out
        .iter()
        .filter_map(|s| match s.datum {
            Datum::Scalar(v) => Some((format!("{}{}", s.id.name, s.id.label), v)),
            _ => None,
        })
        .collect();
    for (k, v) in &scalars {
        if k.starts_with("cpu.core_pct") || k == "cpu.total_pct" {
            assert!((0.0..=100.0).contains(v), "{k} = {v}");
        }
        if k.starts_with("cpu.freq_mhz") {
            assert!((100.0..=9000.0).contains(v), "{k} = {v}");
        }
        if k.starts_with("sensor.temp_c") {
            assert!((0.0..=125.0).contains(v), "{k} = {v}");
        }
    }
    // Scan cost for the PERFORMANCE row: the meters pass is /proc/stat +
    // meminfo + loadavg + uptime + 3 PSI files + a /proc readdir + 32
    // scaling_cur_freq + 3 k10temp inputs.
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    let runs = 20;
    for _ in 0..runs {
        let t = std::time::Instant::now();
        let _ = s.sample(Ts(2), Detail::Meters).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);
        total += ms;
    }
    println!(
        "meters scan: mean {:.2} ms, worst {worst:.2} ms",
        total / f64::from(runs)
    );
    assert!(
        worst < 20.0,
        "a meters scan took {worst:.2} ms (P15 budgets 20 ms for the far bigger pid-level scan)"
    );

    let topo = s.topology();
    assert!(!topo.is_empty(), "sysfs topology must resolve on torch");
    for (k, v) in scalars {
        println!("{k} = {v}");
    }
    println!("dies: {:?}", topo.dies());
}
