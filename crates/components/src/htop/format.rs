//! htop's printed formats (`Meter_humanUnit`, `Row_printTime`, the load and
//! tasks lines), kept apart from the view builders so they can be read against
//! htop 3.4.1's source line by line.

/// htop's `Meter_humanUnit`: K/M/G/T with 2/1/0 decimals below 10/100/∞.
pub fn human_bytes(b: f64) -> String {
    const UNITS: [&str; 5] = ["K", "M", "G", "T", "P"];
    if !b.is_finite() || b < 0.0 {
        return "—".into();
    }
    let mut v = b / 1024.0; // htop prints from KiB up
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    let suffix = UNITS[unit];
    if v < 10.0 {
        format!("{v:.2}{suffix}")
    } else if v < 100.0 {
        format!("{v:.1}{suffix}")
    } else {
        format!("{v:.0}{suffix}")
    }
}

/// htop's uptime meter: `N days(!), HH:MM:SS`, shortened for narrow tiles.
pub fn uptime(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "—".into();
    }
    let s = secs as u64;
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d}d {h:02}:{m:02}")
    } else {
        format!("{h:02}:{m:02}:{:02}", s % 60)
    }
}

/// A percentage in htop's width: `45.2%`, `100%`, and `—` when there is no
/// delta yet (the first scan of a source publishes no percentage at all).
pub fn pct(v: Option<f64>) -> String {
    match v {
        None => "—".into(),
        Some(v) if v >= 99.95 => "100%".into(),
        Some(v) => format!("{v:.1}%"),
    }
}

/// The compact form for a 1x1 tile: `45%`.
pub fn pct_short(v: Option<f64>) -> String {
    match v {
        None => "—".into(),
        Some(v) => format!("{:.0}%", v.clamp(0.0, 100.0)),
    }
}

/// `4.9 GHz` / `4.9G` — the block header's frequency.
pub fn ghz(mhz: f64, short: bool) -> String {
    if !mhz.is_finite() || mhz <= 0.0 {
        return "—".into();
    }
    if short {
        format!("{:.1}G", mhz / 1000.0)
    } else {
        format!("{:.1} GHz", mhz / 1000.0)
    }
}

pub fn celsius(c: f64, short: bool) -> String {
    if !c.is_finite() {
        return "—".into();
    }
    if short {
        format!("{c:.0}°")
    } else {
        format!("{c:.1} °C")
    }
}

/// The task line. **Deliberately not htop's wording**: htop's Tasks meter
/// prints userland processes, userland threads and kernel threads, and all
/// three need the `PF_KTHREAD` read from every `/proc/<pid>/stat` that
/// `Detail::Table` gates in arc 2. What arc 1b can count without that scan is
/// every pid directory and every task, so that is what it says (PARITY.md).
pub fn tasks(pids: Option<f64>, tasks: Option<f64>, running: Option<f64>) -> String {
    let n = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
    format!(
        "{} pids, {} tasks; {} running",
        n(pids),
        n(tasks),
        n(running)
    )
}

pub fn load(one: Option<f64>, five: Option<f64>, fifteen: Option<f64>) -> String {
    let n = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into());
    format!("{} {} {}", n(one), n(five), n(fifteen))
}

// ───────────────────────── the process table (§8.1) ─────────────────────────
//
// `Row_printKBytes`, `Row_printTime` and `Row_printPercentage` from htop
// 3.4.1's `Row.c`, reproduced branch for branch. htop's colours become theme
// roles by meaning: PROCESS → Text, PROCESS_SHADOW → TextMuted,
// PROCESS_MEGABYTES (cyan) → Info, PROCESS_GIGABYTES (green) → Ok,
// LARGE_NUMBER (red) → Crit. Widths here are the *printed* cell without the
// trailing separator space htop appends (the renderer adds it).

use gridwatch_ui::theme::Role;
use gridwatch_ui::view::Span;

/// htop's unit colour ladder: plain, M, G, T+.
const UNIT_ROLES: [Role; 4] = [Role::Text, Role::Info, Role::Ok, Role::Crit];
const UNIT_PREFIXES: [char; 8] = ['K', 'M', 'G', 'T', 'P', 'E', 'Z', 'Y'];

/// `Row_printKBytes`: five cells. `< 1000` plain; `1000–99 999` as five KiB
/// digits with the leading thousands in the M role and no unit (`28248`);
/// `≥ 100 000` as three significant digits plus a unit (`97.6M`, `9.76G`,
/// ` 100M`, `1000M`) with the roles cycling Text → M → G → Large.
pub fn kbytes(kib: Option<u64>) -> Vec<Span> {
    let Some(number) = kib else {
        return vec![Span::new(Role::TextMuted, "  N/A")];
    };
    if number < 1000 {
        return vec![Span::new(Role::Text, format!("{number:5}"))];
    }
    if number < 100_000 {
        return vec![
            Span::new(Role::Info, format!("{:2}", number / 1000)),
            Span::new(Role::Text, format!("{:03}", number % 1000)),
        ];
    }
    // KiB → hundredths of a MiB, exactly as htop does it in integers.
    let mut hundredths: u64 = (number / 256) * 25 + (number % 256) * 25 / 256;
    let mut i = 1usize;
    let mut color = UNIT_ROLES[0];
    let mut next = UNIT_ROLES[1];
    let mut prev;
    loop {
        prev = color;
        color = next;
        if i + 1 < UNIT_ROLES.len() {
            next = UNIT_ROLES[i + 1];
        }
        if hundredths < 1_000_000 {
            break;
        }
        hundredths /= 1024;
        i += 1;
        if i >= UNIT_PREFIXES.len() {
            return vec![Span::new(Role::TextMuted, "  N/A")];
        }
    }
    let whole = hundredths / 100;
    let frac = hundredths % 100;
    let unit = UNIT_PREFIXES[i];
    if whole < 100 {
        if whole < 10 {
            vec![
                Span::new(color, format!("{whole:1}")),
                Span::new(prev, format!(".{frac:02}")),
                Span::new(color, unit.to_string()),
            ]
        } else {
            vec![
                Span::new(color, format!("{whole:2}")),
                Span::new(prev, format!(".{:1}", frac / 10)),
                Span::new(color, unit.to_string()),
            ]
        }
    } else if whole < 1000 {
        vec![Span::new(color, format!("{whole:4}{unit}"))]
    } else {
        vec![
            Span::new(next, format!("{:1}", whole / 1000)),
            Span::new(color, format!("{:03}{unit}", whole % 1000)),
        ]
    }
}

/// `Row_printTime`: eight cells — `MM:SS.hh`, `HHhMM:SS`, `DdHHhMMm`,
/// `DDDDdHHh`, `YYYyDDDd`; hours in the M role, days in G, years in Large.
pub fn time_plus(total_cs: u64) -> Vec<Span> {
    if total_cs == 0 {
        return vec![Span::new(Role::TextMuted, " 0:00.00")];
    }
    let total_seconds = total_cs / 100;
    let total_minutes = total_seconds / 60;
    let total_hours = total_minutes / 60;
    let seconds = total_seconds % 60;
    let minutes = total_minutes % 60;
    if total_minutes < 60 {
        return vec![Span::new(
            Role::Text,
            format!("{total_minutes:2}:{seconds:02}.{:02}", total_cs % 100),
        )];
    }
    if total_hours < 24 {
        return vec![
            Span::new(Role::Info, format!("{total_hours:2}h")),
            Span::new(Role::Text, format!("{minutes:02}:{seconds:02}")),
        ];
    }
    let total_days = total_hours / 24;
    let hours = total_hours % 24;
    if total_days < 10 {
        return vec![
            Span::new(Role::Ok, format!("{total_days:1}d")),
            Span::new(Role::Info, format!("{hours:02}h")),
            Span::new(Role::Text, format!("{minutes:02}m")),
        ];
    }
    if total_days < 365 {
        return vec![
            Span::new(Role::Ok, format!("{total_days:4}d")),
            Span::new(Role::Info, format!("{hours:02}h")),
        ];
    }
    let years = total_days / 365;
    let days = total_days % 365;
    if years < 1000 {
        vec![
            Span::new(Role::Crit, format!("{years:3}y")),
            Span::new(Role::Ok, format!("{days:03}d")),
        ]
    } else if years < 10_000_000 {
        vec![Span::new(Role::Crit, format!("{years:7}y"))]
    } else {
        vec![Span::new(Role::Crit, "eternity")]
    }
}

/// `Row_printPercentage` at `width`: `< 0.05` muted, `≥ 99.9` in the accent
/// (htop's PROCESS_MEGABYTES); a 4-wide column prints `100` above 99.9.
pub fn percentage(v: f32, width: usize) -> Span {
    if v.is_nan() || v < 0.0 {
        return Span::new(Role::TextMuted, format!("{:>width$}", "N/A"));
    }
    let role = if v < 0.05 {
        Role::TextMuted
    } else if v >= 99.9 {
        Role::AccentSecondary
    } else {
        Role::Text
    };
    if width == 4 && v > 99.9 {
        return Span::new(role, format!("{:>width$}", "100"));
    }
    Span::new(role, format!("{v:>width$.1}"))
}

/// htop's process-state colouring (§8.1): `R`/`U`/`t` → Ok, `D`/`Z`/`T`/`X`/`B`
/// → Crit, `S`/`I`/`W`/`Q` → muted, everything else plain. `P` (parked) is
/// htop's `BLOCKED`: it prints as `B` and takes the D-state colour.
pub fn state(c: char) -> Span {
    let shown = if c == 'P' { 'B' } else { c };
    let role = match c {
        'R' | 'U' | 't' => Role::Ok,
        'D' | 'Z' | 'T' | 'X' | 'B' | 'P' => Role::Crit,
        'S' | 'I' | 'W' | 'Q' => Role::TextMuted,
        _ => Role::Text,
    };
    Span::new(role, shown.to_string())
}

/// htop's Tasks meter wording once `kthr` exists (PARITY.md):
/// `{procs}, {uthreads} thr, {kthreads} kthr; {running} running`, where
/// `procs = pids − kthreads` and `uthreads = tasks − pids` — the same
/// arithmetic htop does from its own scan (`totalTasks − userlandThreads −
/// kernelThreads`).
pub fn tasks_htop(pids: f64, tasks: f64, kthreads: f64, running: Option<f64>) -> String {
    let procs = (pids - kthreads).max(0.0);
    let uthreads = (tasks - pids).max(0.0);
    let running = running
        .map(|v| format!("{v:.0}"))
        .unwrap_or_else(|| "—".into());
    format!("{procs:.0}, {uthreads:.0} thr, {kthreads:.0} kthr; {running} running")
}

/// The task line that fits `width`: htop's full form, then clause by clause
/// from the right (`; N running`, then ` kthr`), then the compact `pids`
/// form — never a line clipped mid-word (the `meters` tier is 30 wide at its
/// minimum and htop's wording is 34).
pub fn tasks_fit(
    pids: Option<f64>,
    all_tasks: Option<f64>,
    kthreads: Option<f64>,
    running: Option<f64>,
    width: usize,
) -> String {
    let tasks = all_tasks;
    let n = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
    let full = match (pids, tasks, kthreads) {
        (Some(p), Some(t), Some(k)) => tasks_htop(p, t, k, running),
        _ => self::tasks(pids, tasks, running),
    };
    if full.chars().count() <= width {
        return full;
    }
    if let (Some(p), Some(t), Some(k)) = (pids, tasks, kthreads) {
        let procs = (p - k).max(0.0);
        let uthreads = (t - p).max(0.0);
        for candidate in [
            format!("{procs:.0}, {uthreads:.0} thr, {k:.0} kthr"),
            format!("{procs:.0}, {uthreads:.0} thr; {} run", n(running)),
            format!("{procs:.0}, {uthreads:.0} thr"),
        ] {
            if candidate.chars().count() <= width {
                return candidate;
            }
        }
    }
    let compact = format!("{} pids; {} run", n(pids), n(running));
    if compact.chars().count() <= width {
        return compact;
    }
    format!("{} pids", n(pids))
}
