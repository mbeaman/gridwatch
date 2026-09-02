//! nvtop 3.2.0's number formats (digest §1): byte rates in 1024 steps, the
//! `%6uMiB %3u%%` GPU MEM cell, the header's clocks/temp/fan/power line.

use std::borrow::Cow;

/// nvtop's `RX: <n> <unit>B/s`: integer value, units B/s, KiB/s, MiB/s, GiB/s.
pub fn rate(bps: Option<f64>) -> String {
    let Some(b) = bps else {
        return "—".into();
    };
    let units = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
    let mut v = b.max(0.0);
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{} {}", v.round() as u64, units[i])
}

/// The GPU MEM cell: `12579MiB  38%` (`%6uMiB %3u%%`, used / total).
pub fn gpu_mem(vram_b: Option<u64>, total_b: u64) -> String {
    match vram_b {
        Some(b) => {
            let mib = b >> 20;
            // nvtop rounds (`round(100.0 * used / total)`), review-verified
            // against the 3.2.0 sources.
            let pct = if total_b > 0 {
                (b as f64 / total_b as f64 * 100.0).round().min(100.0) as u64
            } else {
                0
            };
            format!("{mib:>6}MiB {pct:>3}%")
        }
        None => format!("{:>14}", "N/A"),
    }
}

/// HOST MEM: RSS in 1024 units, three significant digits, ≤ 9 cells.
pub fn host_mem(kib: Option<u64>) -> String {
    let Some(k) = kib else {
        return "—".into();
    };
    human_kib(k as f64)
}

pub fn human_kib(kib: f64) -> String {
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut v = kib.max(0.0);
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{:.0}{}", v, units[i])
    } else if v >= 10.0 {
        format!("{:.1}{}", v, units[i])
    } else {
        format!("{:.2}{}", v, units[i])
    }
}

/// Bytes as `13.9G` / `464M` for the gauge texts.
pub fn human_bytes(b: f64) -> String {
    let units = ["B", "K", "M", "G", "T"];
    let mut v = b.max(0.0);
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{}{}", v as u64, units[i])
    } else if v >= 100.0 {
        format!("{:.0}{}", v, units[i])
    } else {
        format!("{:.1}{}", v, units[i])
    }
}

/// `%3u%%` — nvtop's percentage cell; `—` padded when absent.
pub fn pct(v: Option<f64>) -> String {
    match v {
        Some(v) => format!("{:>3}%", v.round().clamp(0.0, 999.0) as u64),
        None => format!("{:>4}", "—"),
    }
}

/// CPU cell, 6 wide: `412.0%`.
pub fn cpu(v: Option<f32>) -> String {
    match v {
        Some(v) => format!("{:>5.1}%", v.max(0.0)),
        None => format!("{:>6}", "—"),
    }
}

pub fn mhz(v: Option<f64>) -> Cow<'static, str> {
    match v {
        Some(v) => format!("{}MHz", v.round() as u64).into(),
        None => "—".into(),
    }
}

pub fn celsius(v: Option<f64>) -> Cow<'static, str> {
    match v {
        Some(v) => format!("{}°C", v.round() as i64).into(),
        None => "—".into(),
    }
}

/// `POW 400/600W`; `—` when either side is missing.
pub fn power(w: Option<f64>, limit: Option<f64>) -> String {
    match (w, limit) {
        (Some(w), Some(l)) => format!("{}/{}W", w.round() as u64, l.round() as u64),
        (Some(w), None) => format!("{}W", w.round() as u64),
        _ => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_step_by_1024_like_nvtop() {
        assert_eq!(rate(Some(0.0)), "0 B/s");
        assert_eq!(rate(Some(1023.0)), "1023 B/s");
        assert_eq!(rate(Some(12.0 * 1024.0 * 1024.0)), "12 MiB/s");
        assert_eq!(rate(Some(3.0 * 1024.0_f64.powi(3))), "3 GiB/s");
        assert_eq!(rate(None), "—");
    }

    #[test]
    fn gpu_mem_is_nvtops_fourteen_cells() {
        let total = 32607u64 << 20;
        let cell = gpu_mem(Some(12579u64 << 20), total);
        assert_eq!(cell, " 12579MiB  39%");
        assert_eq!(cell.chars().count(), 14);
        assert_eq!(gpu_mem(None, total).chars().count(), 14);
    }

    #[test]
    fn host_mem_and_cpu_fit_their_columns() {
        assert_eq!(host_mem(Some(13_107_200)), "12.5GiB");
        assert_eq!(host_mem(Some(984_000)), "961MiB");
        assert_eq!(host_mem(Some(890)), "890KiB");
        assert!(host_mem(Some(2 << 30)).chars().count() <= 9, "2 TiB fits");
        assert_eq!(cpu(Some(412.0)), "412.0%");
        assert_eq!(cpu(Some(3.24)), "  3.2%");
        assert_eq!(pct(Some(17.4)), " 17%");
        assert_eq!(power(Some(400.4), Some(600.0)), "400/600W");
    }
}
