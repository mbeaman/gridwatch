//! `/proc/net/dev` (brief arc 7 seam 2): sixteen counters per interface,
//! and the rates they become. Split on the **first colon**, never on
//! whitespace — a name longer than six characters butts against it
//! (`br-6bb7413a559e:      84`, verified on torch).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// One interface's counters at an instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errs: u64,
    pub rx_drop: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errs: u64,
    pub tx_drop: u64,
}

/// The per-second rates between two samples.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_pps: f64,
    pub tx_pps: f64,
    pub rx_drop: f64,
    pub tx_drop: f64,
    pub rx_err: f64,
    pub tx_err: f64,
}

impl Counters {
    /// Rates from the previous sample over `dt`. A counter that went
    /// backwards means the interface was re-created: the delta is zero,
    /// never a negative or a huge number.
    pub fn rates(&self, prev: &Counters, dt: Duration) -> Rates {
        let secs = dt.as_secs_f64().max(1e-3);
        let d = |now: u64, then: u64| now.saturating_sub(then) as f64 / secs;
        Rates {
            rx_bps: d(self.rx_bytes, prev.rx_bytes),
            tx_bps: d(self.tx_bytes, prev.tx_bytes),
            rx_pps: d(self.rx_packets, prev.rx_packets),
            tx_pps: d(self.tx_packets, prev.tx_packets),
            rx_drop: d(self.rx_drop, prev.rx_drop),
            tx_drop: d(self.tx_drop, prev.tx_drop),
            rx_err: d(self.rx_errs, prev.rx_errs),
            tx_err: d(self.tx_errs, prev.tx_errs),
        }
    }
}

/// Parse the whole file: interface name → counters.
pub fn parse(text: &str) -> HashMap<String, Counters> {
    let mut out = HashMap::new();
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let n: Vec<u64> = rest
            .split_whitespace()
            .map(|v| v.parse::<u64>().unwrap_or(0))
            .collect();
        if n.len() < 16 {
            continue;
        }
        out.insert(
            name.to_string(),
            Counters {
                rx_bytes: n[0],
                rx_packets: n[1],
                rx_errs: n[2],
                rx_drop: n[3],
                tx_bytes: n[8],
                tx_packets: n[9],
                tx_errs: n[10],
                tx_drop: n[11],
            },
        );
    }
    out
}

/// Read and parse `<proc>/net/dev`.
pub fn read(proc: &Path) -> HashMap<String, Counters> {
    std::fs::read_to_string(proc.join("net/dev"))
        .map(|t| parse(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net/proc/net/dev");
        std::fs::read_to_string(p).expect("the fixture reads")
    }

    #[test]
    fn parses_torchs_interfaces_including_the_long_name() {
        let m = parse(&fixture());
        assert!(m.contains_key("lo"));
        assert!(m.contains_key("eno1"), "{:?}", m.keys().collect::<Vec<_>>());
        assert!(m.contains_key("wlp7s0"));
        // The name that butts against the colon: splitting on whitespace
        // would have swallowed the first counter with the name.
        let long = m
            .keys()
            .find(|k| k.starts_with("br-"))
            .expect("a bridge with a long name");
        assert!(long.len() > 6, "{long}");
        let lo = m["lo"];
        assert!(lo.rx_bytes > 0 && lo.rx_bytes == lo.tx_bytes, "loopback");
        let wifi = m["wlp7s0"];
        assert_eq!(wifi.rx_bytes, 0, "the radio is down on torch");
    }

    #[test]
    fn rates_are_per_second_and_a_reset_is_zero() {
        let prev = Counters {
            rx_bytes: 1_000,
            rx_packets: 10,
            tx_bytes: 500,
            tx_packets: 5,
            rx_drop: 2,
            ..Counters::default()
        };
        let now = Counters {
            rx_bytes: 3_000,
            rx_packets: 30,
            tx_bytes: 1_500,
            tx_packets: 15,
            rx_drop: 4,
            ..Counters::default()
        };
        let r = now.rates(&prev, Duration::from_secs(2));
        assert_eq!(r.rx_bps, 1_000.0);
        assert_eq!(r.tx_bps, 500.0);
        assert_eq!(r.rx_pps, 10.0);
        assert_eq!(r.rx_drop, 1.0);
        // The interface was re-created: every counter restarted.
        let reset = Counters {
            rx_bytes: 7,
            ..Counters::default()
        };
        let r = reset.rates(&now, Duration::from_secs(1));
        assert_eq!(r.rx_bps, 0.0, "a reset is not a negative rate");
        // A zero interval cannot divide by zero.
        let r = now.rates(&prev, Duration::ZERO);
        assert!(r.rx_bps.is_finite());
    }

    #[test]
    fn junk_lines_are_skipped() {
        assert!(parse("").is_empty());
        assert!(parse("one\ntwo\n").is_empty());
        assert!(parse("h\nh\nnocolon 1 2 3\n").is_empty());
        assert!(parse("h\nh\neth0: 1 2 3\n").is_empty(), "too few counters");
        let ok = parse(&format!("h\nh\neth0: {}\n", vec!["1"; 16].join(" ")));
        assert_eq!(ok["eth0"].rx_bytes, 1);
        assert!(read(Path::new("/nonexistent")).is_empty());
    }
}
