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
