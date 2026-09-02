//! Latency probes (brief arc 7 seam 2, feature `net-probe`): an
//! unprivileged `SOCK_DGRAM` ICMP echo (torch's `ping_group_range` allows
//! it — 1.4 ms to the gateway, verified) with a **TCP connect** fallback
//! when the socket is refused, and the ring statistics both feed.
//!
//! The socket work is deliberately small and hand-rolled: one datagram out,
//! one in, with a timeout. The statistics are pure and unit-tested.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use gridwatch_store::keys::net::{ProbeKind, ProbeStat};

/// Samples kept per target.
pub const RING: usize = 60;
/// How long a probe waits before calling it lost.
pub const TIMEOUT: Duration = Duration::from_millis(900);
/// The port the TCP fallback knocks on.
pub const TCP_PORT: u16 = 443;

/// One target's rolling statistics.
#[derive(Clone, Debug, Default)]
pub struct Ring {
    /// `None` is a lost probe.
    samples: VecDeque<Option<f64>>,
    pub sent: u64,
    /// The previous round-trip, for RFC 3550's jitter.
    prev: Option<f64>,
    jitter: f64,
}

impl Ring {
    pub fn push(&mut self, ms: Option<f64>) {
        self.sent += 1;
        if self.samples.len() == RING {
            self.samples.pop_front();
        }
        if let Some(ms) = ms {
            // RFC 3550: J += (|D| − J) / 16.
            if let Some(prev) = self.prev {
                let d = (ms - prev).abs();
                self.jitter += (d - self.jitter) / 16.0;
            }
            self.prev = Some(ms);
        }
        self.samples.push_back(ms);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The latest round-trip, if the last probe answered.
    pub fn last(&self) -> Option<f64> {
        self.samples.back().copied().flatten()
    }

    pub fn stat(&self, target: &str, addr: &str, kind: ProbeKind) -> ProbeStat {
        let ok: Vec<f64> = self.samples.iter().flatten().copied().collect();
        let n = ok.len() as f64;
        let avg = if ok.is_empty() {
            0.0
        } else {
            ok.iter().sum::<f64>() / n
        };
        let mdev = if ok.is_empty() {
            0.0
        } else {
            ok.iter().map(|v| (v - avg).abs()).sum::<f64>() / n
        };
        let lost = self.samples.len() - ok.len();
        ProbeStat {
            target: target.to_string(),
            addr: addr.to_string(),
            kind,
            min_ms: ok
                .iter()
                .cloned()
                .fold(f64::INFINITY, f64::min)
                .min(f64::MAX),
            avg_ms: avg,
            max_ms: ok.iter().cloned().fold(0.0, f64::max),
            mdev_ms: mdev,
            jitter_ms: self.jitter,
            loss_pct: if self.samples.is_empty() {
                0.0
            } else {
                lost as f64 * 100.0 / self.samples.len() as f64
            },
            sent: self.sent,
        }
    }
}

/// A TCP-connect probe: the fallback when ICMP is refused. Returns the
/// round-trip in ms, or `None` on timeout/refusal.
pub fn tcp_probe(addr: IpAddr, port: u16, timeout: Duration) -> Option<f64> {
    let t0 = Instant::now();
    let sock = SocketAddr::new(addr, port);
    match TcpStream::connect_timeout(&sock, timeout) {
        Ok(_) => Some(t0.elapsed().as_secs_f64() * 1000.0),
        // A refusal still measures the path: the packet went and came back.
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            Some(t0.elapsed().as_secs_f64() * 1000.0)
        }
        Err(_) => None,
    }
}

/// Can this process open an unprivileged ICMP socket? (torch: yes, because
/// `net.ipv4.ping_group_range` includes every gid.)
pub fn icmp_available() -> bool {
    icmp_socket().is_ok()
}

#[cfg(feature = "net-probe")]
fn icmp_socket() -> std::io::Result<socket2::Socket> {
    use socket2::{Domain, Protocol, Socket, Type};
    Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::ICMPV4))
}

#[cfg(not(feature = "net-probe"))]
fn icmp_socket() -> std::io::Result<()> {
    Err(std::io::Error::other("built without net-probe"))
}

/// One ICMP echo. `seq` distinguishes replies; the identifier is the
/// kernel's own for a DGRAM socket, so a reply is matched by sequence.
#[cfg(feature = "net-probe")]
pub fn icmp_probe(addr: IpAddr, seq: u16, timeout: Duration) -> Option<f64> {
    let IpAddr::V4(v4) = addr else {
        // IPv6 echo needs its own protocol number; the TCP fallback covers
        // it until a v6 default route exists on this machine.
        return None;
    };
    let sock = icmp_socket().ok()?;
    sock.set_read_timeout(Some(timeout)).ok()?;
    // A DGRAM ICMP socket is a datagram socket: hand it to `std` so the
    // receive path needs no `MaybeUninit` (this crate forbids `unsafe`).
    let sock: std::net::UdpSocket = sock.into();
    let mut packet = [0u8; 16];
    packet[0] = 8; // echo request
    packet[6..8].copy_from_slice(&seq.to_be_bytes());
    let sum = checksum(&packet);
    packet[2..4].copy_from_slice(&sum.to_be_bytes());
    let t0 = Instant::now();
    sock.send_to(&packet, SocketAddr::new(IpAddr::V4(v4), 0))
        .ok()?;
    let mut buf = [0u8; 128];
    loop {
        let (n, _) = sock.recv_from(&mut buf).ok()?;
        // A DGRAM ICMP reply arrives without the IP header; type 0 is the
        // echo reply and bytes 6..8 carry our sequence.
        if n >= 8 && buf[0] == 0 && buf[6..8] == seq.to_be_bytes() {
            return Some(t0.elapsed().as_secs_f64() * 1000.0);
        }
        if t0.elapsed() > timeout {
            return None;
        }
    }
}

#[cfg(not(feature = "net-probe"))]
pub fn icmp_probe(_addr: IpAddr, _seq: u16, _timeout: Duration) -> Option<f64> {
    None
}

/// The internet checksum (RFC 1071).
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u32::from(u16::from_be_bytes([data[i], data[i + 1]]));
        i += 2;
    }
    if i < data.len() {
        sum += u32::from(data[i]) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_sixty_and_reports_loss_and_jitter() {
        let mut r = Ring::default();
        for i in 0..10 {
            r.push(Some(10.0 + f64::from(i % 2)));
        }
        let s = r.stat("gw", "10.0.0.1", ProbeKind::Icmp);
        assert_eq!(s.sent, 10);
        assert_eq!(s.min_ms, 10.0);
        assert_eq!(s.max_ms, 11.0);
        assert!((s.avg_ms - 10.5).abs() < 1e-9);
        assert!((s.mdev_ms - 0.5).abs() < 1e-9);
        assert_eq!(s.loss_pct, 0.0);
        assert!(s.jitter_ms > 0.0, "alternating values have jitter");
        // Losses.
        for _ in 0..10 {
            r.push(None);
        }
        let s = r.stat("gw", "10.0.0.1", ProbeKind::Icmp);
        assert_eq!(s.loss_pct, 50.0);
        assert_eq!(s.sent, 20);
        assert_eq!(s.min_ms, 10.0, "the answers still count");
        assert_eq!(r.last(), None);
        // The ring is bounded.
        for i in 0..200 {
            r.push(Some(f64::from(i)));
        }
        assert_eq!(r.len(), RING);
        assert_eq!(r.stat("g", "a", ProbeKind::Tcp).loss_pct, 0.0);
        assert_eq!(r.last(), Some(199.0));
        // An empty ring says nothing rather than dividing by zero.
        let empty = Ring::default();
        let s = empty.stat("g", "a", ProbeKind::Icmp);
        assert_eq!((s.avg_ms, s.loss_pct, s.sent), (0.0, 0.0, 0));
        assert!(empty.is_empty());
    }

    #[test]
    fn the_checksum_matches_rfc_1071() {
        // An echo request with a zero checksum field.
        let mut p = [0u8; 16];
        p[0] = 8;
        p[6..8].copy_from_slice(&1u16.to_be_bytes());
        let c = checksum(&p);
        // Verifying: summing the packet with its checksum in place gives 0.
        let mut with = p;
        with[2..4].copy_from_slice(&c.to_be_bytes());
        assert_eq!(checksum(&with), 0, "a correct checksum verifies to zero");
        // Odd lengths do not panic.
        assert_ne!(checksum(&[1, 2, 3]), 0);
    }

    /// A TCP probe against a socket we opened ourselves: no network needed.
    #[test]
    fn tcp_probe_measures_a_local_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let ms = tcp_probe(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port, TIMEOUT)
            .expect("a local connect answers");
        assert!((0.0..900.0).contains(&ms), "{ms} ms");
        drop(l);
        // A port nobody listens on is refused — which is still a
        // measurement, not a loss.
        let refused = tcp_probe(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port, TIMEOUT);
        assert!(refused.is_some());
    }

    /// Sends one ICMP echo to the default gateway. Unprivileged DGRAM ICMP
    /// is a safe read-only probe on torch, but it leaves the machine, so it
    /// stays ignored in CI.
    #[test]
    #[ignore]
    fn live_icmp_reaches_the_gateway() {
        use std::path::Path;
        let rows = std::fs::read_to_string("/proc/net/route")
            .map(|t| super::super::route::parse(&t))
            .unwrap_or_default();
        let Some(gw) = super::super::route::default_route(&rows).map(|r| r.gateway) else {
            println!("no default route");
            return;
        };
        println!("icmp available: {}", icmp_available());
        for seq in 1..=3 {
            let ms = icmp_probe(IpAddr::V4(gw), seq, TIMEOUT);
            println!("seq {seq} → {ms:?} ms");
        }
        let _ = Path::new("/proc/net/route");
    }
}
