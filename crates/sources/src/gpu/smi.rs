//! The nvidia-smi CSV tier (§8, §11, digest §6): only when `libnvidia-ml.so.1`
//! cannot be loaded while the binary can. One `--query-gpu` subprocess per
//! second feeds every probe field from a cached row; no process rows, no
//! power trace, no PCIe counters, no fan RPM. Status `Degraded("nvidia-smi
//! fallback")`.

use std::process::Command;
use std::time::{Duration, Instant};

use super::probe::{Fail, Probe, ProcMem, ProcUtil, Static};

/// The query, in `Row`'s field order.
pub const QUERY: &str = "name,driver_version,pci.bus_id,pci.device_id,uuid,vbios_version,\
utilization.gpu,utilization.memory,memory.used,memory.total,temperature.gpu,power.draw,\
power.limit,clocks.gr,clocks.mem,clocks.max.gr,clocks.max.mem,pstate,fan.speed,\
pcie.link.gen.current,pcie.link.width.current,clocks_event_reasons.active,\
encoder.stats.averageFps,temperature.gpu.tlimit";

/// One parsed CSV line; `None` where nvidia-smi printed `[N/A]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub name: String,
    pub driver: String,
    pub bus_id: String,
    pub pci_id: Option<u32>,
    pub uuid: String,
    pub vbios: String,
    pub util: Option<u32>,
    pub memctl: Option<u32>,
    pub mem_used_mib: Option<u64>,
    pub mem_total_mib: Option<u64>,
    pub temp_c: Option<u32>,
    pub power_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub clock_gfx: Option<u32>,
    pub clock_mem: Option<u32>,
    pub clock_gfx_max: Option<u32>,
    pub clock_mem_max: Option<u32>,
    pub pstate: Option<u8>,
    pub fan_pct: Option<u32>,
    pub pcie_gen: Option<u32>,
    pub pcie_width: Option<u32>,
    pub throttle_bits: Option<u64>,
}

fn field(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.is_empty() || s.starts_with("[N/A]") || s == "N/A" {
        None
    } else {
        Some(s)
    }
}

fn num<T: std::str::FromStr>(s: &str) -> Option<T> {
    field(s)?.split_whitespace().next()?.parse().ok()
}

/// Parse one `--format=csv,noheader,nounits` line. Extra trailing fields are
/// ignored so an older nvidia-smi that lacks a column still parses.
pub fn parse_row(line: &str) -> Option<Row> {
    let f: Vec<&str> = line.split(',').map(str::trim).collect();
    if f.len() < 19 {
        return None;
    }
    let pci_id = field(f[3]).and_then(|s| {
        // `0x2B8510DE`: device in the upper 16 bits.
        u32::from_str_radix(s.trim_start_matches("0x"), 16)
            .ok()
            .map(|v| v >> 16)
    });
    Some(Row {
        name: field(f[0]).unwrap_or("").to_string(),
        driver: field(f[1]).unwrap_or("").to_string(),
        bus_id: field(f[2]).unwrap_or("").to_string(),
        pci_id,
        uuid: field(f[4]).unwrap_or("").to_string(),
        vbios: field(f[5]).unwrap_or("").to_string(),
        util: num(f[6]),
        memctl: num(f[7]),
        mem_used_mib: num(f[8]),
        mem_total_mib: num(f[9]),
        temp_c: num(f[10]),
        power_w: num(f[11]),
        power_limit_w: num(f[12]),
        clock_gfx: num(f[13]),
        clock_mem: num(f[14]),
        clock_gfx_max: num(f[15]),
        clock_mem_max: num(f[16]),
        pstate: field(f[17]).and_then(|s| s.trim_start_matches('P').parse().ok()),
        fan_pct: num(f[18]),
        pcie_gen: f.get(19).and_then(|s| num(s)),
        pcie_width: f.get(20).and_then(|s| num(s)),
        throttle_bits: f
            .get(21)
            .and_then(|s| field(s))
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()),
    })
}

/// True when `nvidia-smi` is on the path — the condition for this tier.
pub fn available() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("nvidia-smi").is_file()))
}

pub struct SmiProbe {
    index: u32,
    row: Option<Row>,
    fetched: Option<Instant>,
    period: Duration,
}

impl SmiProbe {
    pub fn new(index: u32) -> SmiProbe {
        SmiProbe {
            index,
            row: None,
            fetched: None,
            period: Duration::from_secs(1),
        }
    }

    fn refresh(&mut self) -> Result<&Row, Fail> {
        let stale = self.fetched.is_none_or(|t| t.elapsed() >= self.period);
        // One subprocess per period, whether or not the last one parsed
        // (review: a failed parse re-spawned nvidia-smi on every probe call).
        if stale {
            // Bounded: `nvidia-smi` takes ≈ 10 ms; a hung driver must not pin
            // the source thread past its stop flag, so the child is polled
            // and killed after two seconds.
            let mut child = Command::new("nvidia-smi")
                .args([
                    &format!("--id={}", self.index),
                    &format!("--query-gpu={QUERY}"),
                    "--format=csv,noheader,nounits",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .spawn()
                .map_err(|e| Fail::Loading(format!("nvidia-smi: {e}")))?;
            let deadline = Instant::now() + Duration::from_secs(2);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(st)) => break Some(st),
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                }
            };
            let out = match status {
                Some(status) => {
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    if let Some(mut o) = child.stdout.take() {
                        let _ = std::io::Read::read_to_end(&mut o, &mut stdout);
                    }
                    if let Some(mut e) = child.stderr.take() {
                        let _ = std::io::Read::read_to_end(&mut e, &mut stderr);
                    }
                    std::process::Output {
                        status,
                        stdout,
                        stderr,
                    }
                }
                None => {
                    self.fetched = Some(Instant::now());
                    return Err(Fail::Other("nvidia-smi did not answer within 2 s".into()));
                }
            };
            self.fetched = Some(Instant::now());
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                return Err(if err.contains("mismatch") {
                    Fail::Mismatch
                } else {
                    Fail::GpuLost
                });
            }
            let text = String::from_utf8_lossy(&out.stdout);
            self.row = text.lines().next().and_then(parse_row);
        }
        self.row
            .as_ref()
            .ok_or_else(|| Fail::Other("nvidia-smi printed nothing parseable".into()))
    }

    fn get<T>(&mut self, f: impl Fn(&Row) -> Option<T>) -> Result<T, Fail> {
        let row = self.refresh()?;
        f(row).ok_or(Fail::NotSupported)
    }
}

impl Probe for SmiProbe {
    fn kind(&self) -> &'static str {
        "nvidia-smi"
    }

    fn static_info(&mut self) -> Result<Static, Fail> {
        let r = self.refresh()?.clone();
        Ok(Static {
            name: r.name,
            driver: r.driver,
            cuda: 0,
            arch: String::new(),
            uuid: r.uuid,
            pci_id: r.pci_id.unwrap_or(0),
            bus_id: r.bus_id,
            vbios: r.vbios,
            cores: None,
            bus_width: None,
            clock_gfx_max_mhz: r.clock_gfx_max,
            clock_mem_max_mhz: r.clock_mem_max,
            temp_slowdown_c: None,
            num_fans: u32::from(r.fan_pct.is_some()),
        })
    }

    fn utilization(&mut self) -> Result<(u32, u32), Fail> {
        self.get(|r| Some((r.util?, r.memctl.unwrap_or(0))))
    }

    fn temperature_c(&mut self) -> Result<u32, Fail> {
        self.get(|r| r.temp_c)
    }

    fn power_w(&mut self) -> Result<f64, Fail> {
        self.get(|r| r.power_w)
    }

    fn power_limit_w(&mut self) -> Result<f64, Fail> {
        self.get(|r| r.power_limit_w)
    }

    fn clock_gfx_mhz(&mut self) -> Result<u32, Fail> {
        self.get(|r| r.clock_gfx)
    }

    fn clock_mem_mhz(&mut self) -> Result<u32, Fail> {
        self.get(|r| r.clock_mem)
    }

    fn pstate(&mut self) -> Result<u8, Fail> {
        self.get(|r| r.pstate)
    }

    fn throttle_bits(&mut self) -> Result<u64, Fail> {
        self.get(|r| r.throttle_bits)
    }

    fn memory_b(&mut self) -> Result<(u64, u64), Fail> {
        self.get(|r| Some((r.mem_used_mib? << 20, r.mem_total_mib? << 20)))
    }

    fn encoder_pct(&mut self) -> Result<u32, Fail> {
        Err(Fail::NotSupported)
    }

    fn decoder_pct(&mut self) -> Result<u32, Fail> {
        Err(Fail::NotSupported)
    }

    fn pcie_link(&mut self) -> Result<(u32, u32), Fail> {
        self.get(|r| Some((r.pcie_gen?, r.pcie_width?)))
    }

    fn pcie_bytes(&mut self) -> Result<(u64, u64), Fail> {
        Err(Fail::NotSupported)
    }

    fn fan_pct(&mut self, fan: u32) -> Result<u32, Fail> {
        if fan != 0 {
            return Err(Fail::NotSupported);
        }
        self.get(|r| r.fan_pct)
    }

    fn fan_rpm(&mut self, _fan: u32) -> Result<u32, Fail> {
        Err(Fail::NotSupported)
    }

    fn power_samples(&mut self, _last_ts: u64) -> Result<Vec<(u64, f32)>, Fail> {
        Err(Fail::NotSupported)
    }

    fn graphics_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
        Err(Fail::NotSupported)
    }

    fn compute_procs(&mut self) -> Result<Vec<ProcMem>, Fail> {
        Err(Fail::NotSupported)
    }

    fn proc_util(&mut self, _last_seen_us: u64) -> Result<Vec<ProcUtil>, Fail> {
        Err(Fail::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "NVIDIA GeForce RTX 5090, 610.57.04, 00000000:01:00.0, 0x2B8510DE, \
GPU-1234, 98.02.2E.80.05, 19, 5, 13856, 32607, 45, 108.21, 600.00, 2220, 7001, 3135, 14001, \
P3, 30, 5, 16, 0x0000000000000004, [N/A], [N/A]";

    #[test]
    fn a_torch_line_parses_field_by_field() {
        let r = parse_row(LINE).expect("parses");
        assert_eq!(r.name, "NVIDIA GeForce RTX 5090");
        assert_eq!(r.pci_id, Some(0x2B85), "device id is the upper half");
        assert_eq!((r.util, r.memctl), (Some(19), Some(5)));
        assert_eq!(
            (r.mem_used_mib, r.mem_total_mib),
            (Some(13856), Some(32607))
        );
        assert_eq!(r.power_w, Some(108.21));
        assert_eq!(r.pstate, Some(3));
        assert_eq!((r.pcie_gen, r.pcie_width), (Some(5), Some(16)));
        assert_eq!(r.throttle_bits, Some(4));
    }

    #[test]
    fn n_a_is_absent_not_zero_and_short_lines_are_refused() {
        let line = LINE.replace("19, 5,", "[N/A], [N/A],");
        let r = parse_row(&line).unwrap();
        assert_eq!((r.util, r.memctl), (None, None));
        assert!(parse_row("a, b, c").is_none());
    }
}
