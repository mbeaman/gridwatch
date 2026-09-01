//! The capability probe (§11): cheap checks only — file/socket/env existence —
//! time-boxed by construction (no network, no device I/O, no NVML init).

use std::path::Path;

use gridwatch_store::{CapSet, Capability};

fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).exists()))
        .unwrap_or(false)
}

pub fn probe() -> CapSet {
    let mut caps = CapSet::empty();
    let mut set = |cap, ok: bool| {
        if ok {
            caps.insert(cap);
        }
    };
    set(Capability::Procfs, exists("/proc/stat"));
    set(
        Capability::Hwmon,
        Path::new("/sys/class/hwmon")
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false),
    );
    set(
        Capability::Cpufreq,
        exists("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"),
    );
    set(
        Capability::Rapl,
        std::fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj").is_ok(),
    );
    set(
        Capability::Nvml,
        exists("/usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1")
            || exists("/usr/lib64/libnvidia-ml.so.1"),
    );
    set(Capability::I2cNvidia, exists("/dev/i2c-0"));
    set(Capability::AstralExporter, false); // probed live by the pins source (arc 3)
    set(Capability::PwRecord, on_path("pw-record"));
    set(Capability::PipeWireSocket, {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| Path::new(&d).join("pipewire-0").exists())
            .unwrap_or(false)
    });
    set(
        Capability::DbusSession,
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
    );
    set(Capability::PingSocket, {
        std::fs::read_to_string("/proc/sys/net/ipv4/ping_group_range")
            .map(|s| {
                let mut it = s.split_whitespace().filter_map(|v| v.parse::<i64>().ok());
                match (it.next(), it.next()) {
                    (Some(lo), Some(hi)) => lo <= hi && !(lo == 1 && hi == 0),
                    _ => false,
                }
            })
            .unwrap_or(false)
    });
    set(Capability::NetRaw, false);
    set(
        Capability::TrueColor,
        std::env::var("COLORTERM")
            .map(|v| v.contains("truecolor"))
            .unwrap_or(false),
    );
    set(
        Capability::VteGlyphs,
        std::env::var_os("VTE_VERSION").is_some(),
    );
    set(Capability::Mouse, true);
    caps
}

/// `gridwatch doctor` (full table lands in arc 3; the probe itself is arc 1a).
pub fn doctor_lines(caps: &CapSet) -> Vec<String> {
    gridwatch_store::ALL_CAPABILITIES
        .iter()
        .map(|c| format!("{} {c:?}", if caps.has(*c) { "✓" } else { "✗" }))
        .collect()
}
