//! The capability probe (§11): cheap checks only — file/socket/env existence —
//! time-boxed by construction (no network, no device I/O, no NVML init). The
//! live probes a source owns (an exporter GET, `detect_bus`) belong to
//! `gridwatch doctor` and run there, never here (P18).

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
    set(Capability::AstralCsv, false); // the CSV tail is arc 8 (BACKLOG)
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

/// What a capability means when it is present, why it is usually missing,
/// and the fix (§11): the placeholder chip's second line and the doctor's
/// columns come from here, so a person sees the same words in both places.
pub fn explain(cap: Capability) -> (&'static str, &'static str, &'static str) {
    match cap {
        Capability::Procfs => (
            "/proc/stat readable",
            "/proc is not mounted or not readable",
            "mount procfs at /proc",
        ),
        Capability::Hwmon => (
            "/sys/class/hwmon has chips",
            "no hwmon chips under /sys/class/hwmon",
            "load the sensor driver (k10temp, nct6775 …) or run `sensors-detect`",
        ),
        Capability::Cpufreq => (
            "cpufreq scaling_cur_freq present",
            "no cpufreq entry for cpu0",
            "enable the cpufreq driver (amd-pstate / acpi-cpufreq) in the kernel",
        ),
        Capability::Rapl => (
            "RAPL energy counter readable",
            "intel-rapl:0/energy_uj missing or unreadable (root-only on many kernels)",
            "install a udev rule that opens /sys/class/powercap/*/energy_uj to your user, or accept no package power",
        ),
        Capability::Nvml => (
            "libnvidia-ml.so.1 found",
            "libnvidia-ml.so.1 not found",
            "install the NVIDIA driver's compute library (Debian/Ubuntu: libnvidia-compute-*)",
        ),
        Capability::I2cNvidia => (
            "/dev/i2c-* present",
            "no /dev/i2c-* devices",
            "load i2c-dev (`sudo modprobe i2c-dev`) and add yourself to the i2c group: `sudo usermod -aG i2c $USER`",
        ),
        Capability::AstralExporter => (
            "astral-watch exporter answers",
            "no astral-watch exporter answered (the pins source falls back to i2c)",
            "start `astral-watch` with `[export]` enabled, or set `[sources.pins] source = \"i2c\"`",
        ),
        Capability::AstralCsv => (
            "astral-watch CSV log tail",
            "the CSV tail backend is arc 8 (BACKLOG) — not in this build",
            "nothing to do yet",
        ),
        Capability::PwRecord => (
            "pw-record on PATH",
            "pw-record not on PATH",
            "install pipewire-bin (Debian/Ubuntu: `sudo apt install pipewire-bin`)",
        ),
        Capability::PipeWireSocket => (
            "$XDG_RUNTIME_DIR/pipewire-0 present",
            "no PipeWire socket in $XDG_RUNTIME_DIR",
            "start the PipeWire user service: `systemctl --user start pipewire`",
        ),
        Capability::DbusSession => (
            "DBUS_SESSION_BUS_ADDRESS set",
            "no session bus address in the environment",
            "run inside a desktop session, or `export DBUS_SESSION_BUS_ADDRESS` from `dbus-launch`",
        ),
        Capability::PingSocket => (
            "unprivileged ICMP sockets allowed",
            "net.ipv4.ping_group_range excludes your group",
            "`sudo sysctl -w net.ipv4.ping_group_range=\"0 2147483647\"`",
        ),
        Capability::NetRaw => (
            "CAP_NET_RAW held",
            "CAP_NET_RAW is never assumed (arc 7 uses ping sockets instead)",
            "nothing to do",
        ),
        Capability::TrueColor => (
            "COLORTERM advertises truecolor",
            "COLORTERM does not advertise truecolor — colours downsample to 256",
            "use a truecolor terminal (Ptyxis, VTE ≥ 0.36) or set COLORTERM=truecolor",
        ),
        Capability::VteGlyphs => (
            "VTE terminal (native box drawing and octants)",
            "VTE_VERSION not set — box drawing comes from the font",
            "run the glyph check in docs/THEMES.md in your terminal",
        ),
        Capability::Mouse => (
            "mouse reporting assumed",
            "mouse disabled",
            "set `mouse = true`",
        ),
    }
}

/// Reason + fix for a *missing* capability, the placeholder chip's two lines.
pub fn missing_lines(cap: Capability) -> (String, String) {
    let (_, why, fix) = explain(cap);
    (format!("needs {cap:?}: {why}"), format!("fix: {fix}"))
}

/// A live probe result a source contributed (`gridwatch doctor`): the
/// capability, whether it answered, and what it said.
pub type LiveProbe = (Capability, bool, String);

/// `gridwatch doctor`: every capability with `✓`/`✗`, a reason and the fix;
/// a live probe from a source replaces the static row's verdict for that
/// capability (the exporter really was asked, `detect_bus` really ran).
pub fn doctor_lines(caps: &CapSet, live: &[LiveProbe]) -> Vec<String> {
    let mut out = vec![
        format!("{:<2}{:<16}{}", "", "capability", "reason · fix"),
        "─".repeat(72),
    ];
    for c in gridwatch_store::ALL_CAPABILITIES {
        let (ok_why, missing_why, fix) = explain(*c);
        let probed = live.iter().find(|(cap, _, _)| cap == c);
        // A live row says what it saw *and* the fix for that (the source
        // knows why `detect_bus` failed); the static fix is for static rows.
        let (ok, tail) = match probed {
            Some((_, ok, what)) => (*ok, what.clone()),
            None if caps.has(*c) => (true, ok_why.to_string()),
            None => (false, format!("{missing_why} · fix: {fix}")),
        };
        let mark = if ok { "✓" } else { "✗" };
        out.push(format!("{mark} {:<16}{tail}", format!("{c:?}")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text `gridwatch doctor` prints on a host with nothing — every row
    /// says why and how, and the words match the placeholder chip's.
    #[test]
    fn doctor_on_a_bare_host_names_every_fix() {
        let lines = doctor_lines(&CapSet::empty(), &[]);
        assert_eq!(lines.len(), 2 + gridwatch_store::ALL_CAPABILITIES.len());
        assert!(lines[2].starts_with("✗ Procfs"));
        assert!(
            lines[2].contains("fix: mount procfs at /proc"),
            "{}",
            lines[2]
        );
        let nvml = lines.iter().find(|l| l.contains("Nvml")).unwrap();
        assert!(nvml.contains("libnvidia-compute"), "{nvml}");
        let i2c = lines.iter().find(|l| l.contains("I2cNvidia")).unwrap();
        assert!(i2c.contains("usermod -aG i2c"), "{i2c}");
        let (reason, fix) = missing_lines(Capability::Procfs);
        assert_eq!(reason, "needs Procfs: /proc is not mounted or not readable");
        assert_eq!(fix, "fix: mount procfs at /proc");
    }

    /// A live probe overrides the static verdict for its capability.
    #[test]
    fn live_probes_replace_static_rows() {
        let live = vec![(
            Capability::AstralExporter,
            true,
            "answers at 127.0.0.1:9942 (astral-watch 0.7.0)".to_string(),
        )];
        let lines = doctor_lines(&CapSet::empty(), &live);
        let row = lines.iter().find(|l| l.contains("AstralExporter")).unwrap();
        assert!(row.starts_with("✓"), "{row}");
        assert!(row.contains("answers at"), "{row}");
        // A failed live probe keeps its own words; the static fix is not
        // appended on top (the doctor said "GPU idle" and "usermod" at once).
        let live = vec![(
            Capability::I2cNvidia,
            false,
            "waiting for telemetry (GPU idle?) — retried every 10 s".to_string(),
        )];
        let lines = doctor_lines(&CapSet::empty(), &live);
        let row = lines.iter().find(|l| l.contains("I2cNvidia")).unwrap();
        assert!(row.starts_with("✗"), "{row}");
        assert!(!row.contains("usermod"), "{row}");
    }
}
