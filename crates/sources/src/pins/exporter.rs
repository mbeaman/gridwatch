//! The exporter backend (digest §2b): one `GET /metrics` over a plain
//! `TcpStream` — no HTTP crate — with a 250 ms connect and a 1 s read
//! timeout. Authoritative when the astral-watch service runs (its debounced
//! flags ride along as `service_active`), and it adds no bus traffic.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use astral_watch::decode::Reading;
use gridwatch_store::keys::pins::PinsMode;

use super::backend::{Described, Loss, PinsBackend};
use super::parse::{Scrape, parse_metrics};

pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
pub const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// Fetch `/metrics` from `addr` (`host:port`).
pub fn fetch(addr: &str) -> Result<String, Loss> {
    let sock = addr
        .to_socket_addrs()
        .map_err(|e| Loss::Unreachable(format!("{addr}: {e}")))?
        .next()
        .ok_or_else(|| Loss::Unreachable(format!("{addr}: no address")))?;
    let mut s = TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)
        .map_err(|e| Loss::Unreachable(format!("{addr}: {e}")))?;
    let _ = s.set_read_timeout(Some(READ_TIMEOUT));
    let _ = s.set_write_timeout(Some(READ_TIMEOUT));
    s.write_all(b"GET /metrics HTTP/1.1\r\nHost: astral-watch\r\nConnection: close\r\n\r\n")
        .map_err(|e| Loss::Unreachable(e.to_string()))?;
    let mut buf = String::new();
    s.read_to_string(&mut buf)
        .map_err(|e| Loss::Unreachable(e.to_string()))?;
    // Body after the blank line; a non-200 is "not found" (the service
    // answers 404 on any other path, so the endpoint is simply not there).
    let (head, body) = buf
        .split_once("\r\n\r\n")
        .ok_or_else(|| Loss::Unreachable("no HTTP header".into()))?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(Loss::NotFound);
    }
    Ok(body.to_string())
}

pub struct ExporterBackend {
    addr: String,
    interval: Duration,
    /// Consecutive connect failures (shown nowhere yet; kept for the status).
    #[allow(dead_code)]
    failures: u32,
    last: Scrape,
}

impl ExporterBackend {
    pub fn new(addr: &str, interval: Duration) -> ExporterBackend {
        ExporterBackend {
            addr: addr.to_string(),
            interval,
            failures: 0,
            last: Scrape::default(),
        }
    }

    /// True when the endpoint answers right now — the `auto` probe.
    pub fn reachable(addr: &str) -> bool {
        fetch(addr).is_ok()
    }

    /// The staleness rule (digest §2b): `up == 0` or an age over 3 × interval
    /// is no telemetry.
    pub fn judge(s: &Scrape, interval: Duration) -> Result<Reading, Loss> {
        if !s.up {
            return Err(Loss::Implausible);
        }
        if let Some(age) = s.age_s
            && age > 3.0 * interval.as_secs_f64()
        {
            return Err(Loss::Implausible);
        }
        s.reading.ok_or(Loss::Implausible)
    }
}

impl PinsBackend for ExporterBackend {
    fn kind(&self) -> PinsMode {
        PinsMode::Exporter
    }

    fn describe(&mut self) -> Result<Described, Loss> {
        Ok(Described {
            bus: None,
            addr: 0,
            // In exporter mode `pci` carries the endpoint and `access` is
            // `"http"` — documented on `PinsInfo` (review).
            pci: self.addr.clone(),
            model: self
                .last
                .version
                .as_ref()
                .map(|v| format!("astral-watch {v}")),
            access: "http".into(),
        })
    }

    fn read(&mut self) -> Result<Reading, Loss> {
        match fetch(&self.addr) {
            Ok(body) => {
                self.failures = 0;
                self.last = parse_metrics(&body);
                Self::judge(&self.last, self.interval)
            }
            // A non-200 answer: the endpoint is really gone → hand over.
            Err(Loss::NotFound) => Err(Loss::NotFound),
            // Connection failures are losses the lifecycle absorbs (a service
            // restarting); the brief's "two failures ⇒ down" is the *status*,
            // not a hand-over (review) — the misses counter carries it.
            Err(e) => {
                self.failures += 1;
                Err(e)
            }
        }
    }

    fn service_active(&self) -> Vec<String> {
        self.last.active.clone()
    }

    fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }
}
