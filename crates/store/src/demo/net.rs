//! Deterministic network synthesis (§12.5, brief arc 7): torch's own three
//! interesting interfaces — an up 1 GbE (`eno1`) carrying a plausible
//! download, a Wi-Fi radio that comes and goes (so the link-flap path is
//! exercised), a bridge with no carrier — plus a connection table, the
//! default route and two probe targets. Byte-deterministic per `(seed, Ts)`.

use std::sync::Arc;
use std::time::Duration;

use crate::demo::XorShift;
use crate::key::{Datum, Label, MetricId};
use crate::keys::net::{
    self, Conn, Conns, Link, LinkKind, ProbeKind, ProbeStat, Probes, Proto, Route, WifiInfo,
};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Detail, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

/// The Wi-Fi link drops for ten seconds out of every sixty.
pub const WIFI_DOWN_FROM_S: f64 = 40.0;
pub const WIFI_DOWN_TO_S: f64 = 50.0;
pub const CYCLE_S: f64 = 60.0;

fn named(key: &crate::key::Key<f64>, label: &str) -> MetricId {
    MetricId {
        name: key.id.name,
        label: Label::Name(Arc::from(label)),
    }
}

#[derive(Clone, Debug)]
pub struct NetSynth {
    rng: XorShift,
    links_sent: bool,
    route_sent: bool,
    wifi_was_up: Option<bool>,
}

/// Is the synthetic Wi-Fi link up at `at`?
pub fn wifi_up(at: Ts) -> bool {
    let t = at.as_secs_f64() % CYCLE_S;
    !(WIFI_DOWN_FROM_S..WIFI_DOWN_TO_S).contains(&t)
}

fn eno1_link() -> Link {
    Link {
        iface: "eno1".into(),
        up: true,
        carrier: true,
        operstate: "up".into(),
        mtu: 1500,
        mac: "a8:a1:59:00:00:01".into(),
        kind: LinkKind::Ether,
        speed_mbps: 1000,
        carrier_changes: 2,
        addrs: vec!["192.168.100.154/24".into(), "fe80::1/64".into()],
        wifi: None,
    }
}

fn wifi_link(up: bool) -> Link {
    Link {
        iface: "wlp7s0".into(),
        up,
        carrier: up,
        operstate: if up { "up".into() } else { "down".into() },
        mtu: 1500,
        mac: "a8:a1:59:00:00:02".into(),
        kind: LinkKind::Wifi,
        // The driver refuses `speed` on a down radio: never a number.
        speed_mbps: if up { 866 } else { net::SPEED_UNKNOWN as i64 },
        carrier_changes: 9,
        addrs: if up {
            vec!["192.168.100.77/24".into()]
        } else {
            Vec::new()
        },
        wifi: up.then(|| WifiInfo {
            ssid: "torchnet".into(),
            signal_dbm: -52,
            freq_mhz: 5180,
            rx_bitrate_kbps: 866_000,
            tx_bitrate_kbps: 780_000,
        }),
    }
}

fn bridge_link() -> Link {
    Link {
        iface: "br-6bb7413a559e".into(),
        up: true,
        carrier: false,
        operstate: "down".into(),
        mtu: 1500,
        mac: "02:42:9c:00:00:03".into(),
        kind: LinkKind::Virtual,
        speed_mbps: net::SPEED_UNKNOWN as i64,
        carrier_changes: 0,
        addrs: vec!["172.18.0.1/16".into()],
        wifi: None,
    }
}

/// The interfaces a workstation actually has beyond the three interesting
/// ones: a second NIC nobody plugged in, and two virtual bridges. They exist
/// so the tile's default filter has something to drop — with three interfaces
/// and one bridge, a filtered tile and an unfiltered one differ by a single
/// row, which is not a difference any test can see (arc 16, D67 §4).
fn quiet_links() -> [Link; 3] {
    let virt = |iface: &str, mac: &str, addr: &str| Link {
        iface: iface.into(),
        up: true,
        carrier: true,
        operstate: "up".into(),
        mtu: 1500,
        mac: mac.into(),
        kind: LinkKind::Virtual,
        speed_mbps: net::SPEED_UNKNOWN as i64,
        carrier_changes: 0,
        addrs: vec![addr.into()],
        wifi: None,
    };
    [
        Link {
            iface: "eno2".into(),
            up: false,
            carrier: false,
            operstate: "down".into(),
            mtu: 1500,
            mac: "a8:a1:59:00:00:04".into(),
            kind: LinkKind::Ether,
            // Nothing is plugged in: the driver refuses `speed`, as on a down
            // radio. A second NIC reading "0 Mb/s" would be a lie.
            speed_mbps: net::SPEED_UNKNOWN as i64,
            carrier_changes: 0,
            addrs: Vec::new(),
            wifi: None,
        },
        virt("docker0", "02:42:1a:00:00:05", "172.17.0.1/16"),
        virt("virbr0", "52:54:00:00:00:06", "192.168.122.1/24"),
    ]
}

/// The connection table the journal test uses as its exemplar.
pub fn conns_exemplar() -> Conns {
    conns(3.0)
}

/// The synthetic connection table (the `conns` tier's rows).
///
/// **`scanned` and `attributed` are derived, never asserted.** The real source
/// sets `scanned = rows.len()` (`sources/src/net/mod.rs`) — it reads every
/// socket and publishes every row, with no cap and no truncation — and
/// `attributed` is the count that resolved to a pid. So a fixture claiming
/// `scanned: 103` beside five rows described a source that does not exist, and
/// the tile's own footer printed it (arc 16, D67 §4). They are computed here
/// and pinned by `conns_counters_are_what_the_real_source_would_report`.
///
/// Thirty-two rows, because the `conns` tier's connection band is at least 23
/// and the fold has to be reachable: with five rows no cursor could ever leave
/// the first page, which is why D65 recorded the pty case for it as unwritable
/// against the shipped fixture.
pub fn conns(scan_ms: f64) -> Conns {
    let row =
        |proto: Proto, local: &str, remote: &str, state: &str, pid: Option<u32>, process: &str| {
            Conn {
                proto,
                local: local.into(),
                remote: remote.into(),
                state: state.into(),
                inode: 1000 + u64::from(pid.unwrap_or(0)),
                pid,
                uid: 1000,
                process: process.into(),
            }
        };
    // A plausible desktop, in the order a socket walk actually returns them —
    // **interleaved, not grouped by process**. `/proc/net/tcp` is ordered by
    // hash bucket, and a fixture that lists eight firefox rows first puts
    // every unattributed socket past the fold, where the first page cannot
    // show that `attributed < scanned` is a thing the tile draws (arc 16).
    // Where two rows share a prefix, the ports differ in their leading
    // digits: `local` is elastic and truncates with no ellipsis below ~82
    // cells, and the first draft's `:41022`/`:41026` rendered as one string
    // at 80 columns — two sockets a reader cannot tell apart (arc 16 review).
    // The eight firefox `:523xx` sockets collide the same way and are a
    // backlog item, because the real fix is an ellipsis in the renderer.
    // States are `conns::state_name`'s spelling — **hyphens, not underscores**.
    // The first draft of this fixture wrote `TIME_WAIT`, `CLOSE_WAIT` and
    // `SYN_SENT`, which no kernel path can produce; `CLOSE-WAIT` is also the
    // one state that overran the tile's `state` column (arc 16 review, S1).
    let rows = vec![
        row(
            Proto::Tcp,
            "192.168.100.154:52344",
            "140.82.112.4:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(Proto::Tcp, "0.0.0.0:22", "0.0.0.0:0", "LISTEN", None, ""),
        row(
            Proto::Tcp,
            "192.168.100.154:44120",
            "162.159.130.234:443",
            "ESTAB",
            Some(9001),
            "steam",
        ),
        row(Proto::Udp, "192.168.100.154:68", "0.0.0.0:0", "", None, ""),
        row(
            Proto::Tcp,
            "192.168.100.154:52352",
            "140.82.114.26:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Tcp,
            "127.0.0.1:6600",
            "0.0.0.0:0",
            "LISTEN",
            Some(7000),
            "mpd",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:48802",
            "140.82.113.25:22",
            "TIME-WAIT",
            None,
            "",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:39004",
            "13.107.42.14:443",
            "ESTAB",
            Some(5150),
            "code",
        ),
        row(
            Proto::Tcp6,
            "[::1]:6600",
            "[::1]:39422",
            "ESTAB",
            Some(7000),
            "mpd",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:52360",
            "151.101.1.140:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Udp,
            "0.0.0.0:5353",
            "0.0.0.0:0",
            "",
            Some(1122),
            "avahi-daemon",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:44126",
            "155.133.248.53:27020",
            "ESTAB",
            Some(9001),
            "steam",
        ),
        row(Proto::Tcp, "0.0.0.0:5355", "0.0.0.0:0", "LISTEN", None, ""),
        row(
            Proto::Tcp,
            "192.168.100.154:60122",
            "192.168.100.30:22",
            "ESTAB",
            Some(8810),
            "ssh",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:52368",
            "34.107.221.82:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Udp,
            "127.0.0.1:53",
            "0.0.0.0:0",
            "",
            Some(1240),
            "systemd-resolve",
        ),
        row(Proto::Tcp6, "[::]:22", "[::]:0", "LISTEN", None, ""),
        row(
            Proto::Tcp,
            "127.0.0.1:631",
            "0.0.0.0:0",
            "LISTEN",
            Some(1430),
            "cupsd",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:48806",
            "140.82.113.25:22",
            "CLOSE-WAIT",
            None,
            "",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:52376",
            "142.250.187.206:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:33518",
            "192.168.100.12:5432",
            "ESTAB",
            Some(6120),
            "psql",
        ),
        row(
            Proto::Udp,
            "192.168.100.154:51820",
            "203.0.113.7:51820",
            "",
            None,
            "",
        ),
        row(
            Proto::Tcp,
            "127.0.0.1:9942",
            "0.0.0.0:0",
            "LISTEN",
            Some(3311),
            "astral-watch",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:52384",
            "104.18.32.47:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:44132",
            "155.133.246.68:27018",
            "ESTAB",
            Some(9001),
            "steam",
        ),
        row(
            Proto::Tcp6,
            "[::1]:631",
            "[::]:0",
            "LISTEN",
            Some(1430),
            "cupsd",
        ),
        row(
            Proto::Udp,
            "0.0.0.0:27036",
            "0.0.0.0:0",
            "",
            Some(9001),
            "steam",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:52392",
            "104.18.33.47:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Tcp,
            "0.0.0.0:3000",
            "0.0.0.0:0",
            "LISTEN",
            Some(5150),
            "code",
        ),
        row(
            Proto::Tcp6,
            "[2001:db8::154]:41022",
            "[2606:4700::47]:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
        row(
            Proto::Tcp,
            "192.168.100.154:52400",
            "140.82.112.21:443",
            "SYN-SENT",
            None,
            "",
        ),
        row(
            Proto::Tcp6,
            "[2001:db8::154]:49318",
            "[2606:4700::48]:443",
            "ESTAB",
            Some(4242),
            "firefox",
        ),
    ];
    let attributed = rows.iter().filter(|r| r.pid.is_some()).count();
    Conns {
        scanned: rows.len(),
        attributed,
        rows,
        scan_ms,
    }
}

fn route() -> Route {
    Route {
        default_iface: "eno1".into(),
        gateway: "192.168.100.1".into(),
        prefsrc: "192.168.100.154".into(),
        dns: vec!["192.168.100.1".into()],
        public_ip: None,
    }
}

impl NetSynth {
    pub fn new(seed: u64) -> NetSynth {
        NetSynth {
            rng: XorShift::new(seed.wrapping_add(0x006e_6574)),
            links_sent: false,
            route_sent: false,
            wifi_was_up: None,
        }
    }

    /// Bytes per second on `eno1` at `at`: a slow swell with jitter, so a
    /// sparkline has something to draw.
    fn rates(&mut self, at: Ts) -> (f64, f64) {
        let t = at.as_secs_f64();
        let swell = 0.5 + 0.5 * ((t / 25.0) * std::f64::consts::TAU).sin();
        let rx = 2.0e6 + 9.0e6 * swell + self.rng.f64() * 3.0e5;
        let tx = 1.4e5 + 6.0e5 * swell + self.rng.f64() * 2.0e4;
        ((rx / 64.0).round() * 64.0, (tx / 64.0).round() * 64.0)
    }

    pub fn tick_at(&mut self, at: Ts, detail: Detail) -> Batch {
        let mut samples = Vec::with_capacity(24);
        let (rx, tx) = self.rates(at);
        let up = wifi_up(at);
        // eno1 carries the traffic; the radio carries a trickle while up.
        // docker0 carries a trickle — a container talking to the host is the
        // ordinary case, and a virtual interface pinned at exactly zero made
        // every mirrored chart line sit on the baseline.
        for (iface, rx, tx) in [
            ("eno1", rx, tx),
            (
                "wlp7s0",
                if up { 4.0e4 } else { 0.0 },
                if up { 9.0e3 } else { 0.0 },
            ),
            ("br-6bb7413a559e", 0.0, 0.0),
            ("eno2", 0.0, 0.0),
            ("docker0", 1.2e4 + self.rng.f64() * 2.0e3, 3.1e3),
            ("virbr0", 0.0, 0.0),
        ] {
            samples.push(Sample {
                id: named(&net::RX_BPS, iface),
                datum: Datum::Scalar(rx),
            });
            samples.push(Sample {
                id: named(&net::TX_BPS, iface),
                datum: Datum::Scalar(tx),
            });
            samples.push(Sample {
                id: named(&net::RX_PPS, iface),
                datum: Datum::Scalar((rx / 1400.0).round()),
            });
            samples.push(Sample {
                id: named(&net::TX_PPS, iface),
                datum: Datum::Scalar((tx / 1400.0).round()),
            });
        }
        // A drop every few seconds on the busy interface, so the column is
        // not always zero.
        let drops = if (at.as_secs_f64() % 7.0) < 1.0 {
            1.0
        } else {
            0.0
        };
        samples.push(Sample {
            id: named(&net::RX_DROP, "eno1"),
            datum: Datum::Scalar(drops),
        });
        let links_changed = self.wifi_was_up != Some(up);
        if !self.links_sent || links_changed {
            self.links_sent = true;
            self.wifi_was_up = Some(up);
            let inventory = [eno1_link(), wifi_link(up), bridge_link()]
                .into_iter()
                .chain(quiet_links());
            for link in inventory {
                samples.push(Sample {
                    id: named(&net::SPEED_MBPS, &link.iface),
                    datum: Datum::Scalar(link.speed_mbps as f64),
                });
                samples.push(Sample {
                    id: MetricId {
                        name: net::LINK.id.name,
                        label: Label::Name(Arc::from(link.iface.as_str())),
                    },
                    datum: Datum::Record(Arc::new(link)),
                });
            }
        }
        if !self.route_sent {
            self.route_sent = true;
            samples.push(Sample {
                id: net::ROUTE.id.clone(),
                datum: Datum::Record(Arc::new(route())),
            });
        }
        // The probes: the gateway is fast and steady, the internet target
        // wanders and loses the odd packet.
        let mut targets = Vec::with_capacity(2);
        for (name, addr, base, spread) in [
            ("gateway", "192.168.100.1", 1.4, 0.3),
            ("1.1.1.1", "1.1.1.1", 11.5, 4.0),
        ] {
            let jitter = self.rng.f64() * spread;
            let rtt = base + jitter;
            let loss = if name == "gateway" {
                0.0
            } else {
                (self.rng.f64() * 3.0).floor()
            };
            samples.push(Sample {
                id: named(&net::RTT_MS, name),
                datum: Datum::Scalar((rtt * 100.0).round() / 100.0),
            });
            samples.push(Sample {
                id: named(&net::LOSS_PCT, name),
                datum: Datum::Scalar(loss),
            });
            targets.push(ProbeStat {
                target: name.into(),
                addr: addr.into(),
                kind: ProbeKind::Icmp,
                min_ms: base,
                avg_ms: (rtt * 100.0).round() / 100.0,
                max_ms: base + spread,
                mdev_ms: (spread / 3.0 * 100.0).round() / 100.0,
                jitter_ms: (jitter * 100.0).round() / 100.0,
                loss_pct: loss,
                sent: 60,
            });
        }
        samples.push(Sample {
            id: net::PROBE.id.clone(),
            datum: Datum::Record(Arc::new(Probes {
                targets,
                degraded: None,
            })),
        });
        // The connection table is table-tier work, like the process table.
        if detail >= Detail::Table {
            let scan_ms = 2.8 + self.rng.f64() * 0.6;
            samples.push(Sample {
                id: net::SCAN_MS.id.clone(),
                datum: Datum::Scalar((scan_ms * 10.0).round() / 10.0),
            });
            samples.push(Sample {
                id: net::CONNS.id.clone(),
                datum: Datum::Record(Arc::new(conns((scan_ms * 10.0).round() / 10.0))),
            });
        }
        Batch {
            source: net::SOURCE,
            at,
            samples,
        }
    }
}

/// The net source's static info (§5): 1 s at every level.
pub fn net_info() -> SourceInfo {
    SourceInfo {
        id: net::SOURCE,
        produces: &["net.*"],
        cadence: Cadence {
            hidden: Some(Duration::from_secs(2)),
            visible: Duration::from_secs(1),
            focused: Duration::from_secs(1),
            always_on: false,
        },
        requires: &[],
    }
}

struct NetDemoSource {
    seed: u64,
}

impl Source for NetDemoSource {
    fn info(&self) -> SourceInfo {
        net_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = NetSynth::new(self.seed);
        cx.status(SourceStatus {
            state: SourceState::Ok,
            reason: Some(Arc::from("synthetic (demo)")),
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        });
        loop {
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
            let Some(cadence) = self.info().cadence.for_level(cx.demand.level()) else {
                if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                    return;
                }
                continue;
            };
            if !cx.sleep_until(cx.next_deadline(cadence)) {
                return;
            }
            let at = cx.clock.now();
            let b = synth.tick_at(at, cx.demand.detail());
            cx.emit(at, b.samples);
        }
    }
}

pub fn net_demo(seed: u64) -> Box<dyn Source> {
    Box::new(NetDemoSource { seed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_interfaces_a_flapping_radio_and_a_table_only_at_table_detail() {
        let (mut a, mut b) = (NetSynth::new(5), NetSynth::new(5));
        let mut link_batches = 0;
        for i in 1..=90 {
            let at = Ts(i * 1_000_000_000);
            let x = a.tick_at(at, Detail::Table);
            let y = b.tick_at(at, Detail::Table);
            assert_eq!(x.samples.len(), y.samples.len(), "deterministic at {i}s");
            for (p, q) in x.samples.iter().zip(&y.samples) {
                assert_eq!(p.id, q.id);
                if let (Datum::Scalar(u), Datum::Scalar(v)) = (&p.datum, &q.datum) {
                    assert_eq!(u, v);
                }
            }
            if x.samples.iter().any(|s| s.id.name == "net.link") {
                link_batches += 1;
            }
        }
        // Once at the start, then on each edge of the radio's flap.
        assert!(
            (3..=5).contains(&link_batches),
            "link batches: {link_batches}"
        );
        assert!(wifi_up(Ts(10_000_000_000)));
        assert!(!wifi_up(Ts(45_000_000_000)));
        assert!(wifi_up(Ts(55_000_000_000)));
        // The down radio never reports a speed.
        let down = wifi_link(false);
        assert_eq!(down.speed_mbps, -1);
        assert!(down.wifi.is_none());
        assert_eq!(down.state(), "down");
        assert_eq!(bridge_link().state(), "no carrier");
        // Meters detail: no connection table.
        let meters = NetSynth::new(1).tick_at(Ts(1_000_000_000), Detail::Meters);
        assert!(!meters.samples.iter().any(|s| s.id.name == "net.conns"));
        assert!(meters.samples.iter().any(|s| s.id.name == "net.probe"));
        let table = NetSynth::new(1).tick_at(Ts(1_000_000_000), Detail::Table);
        assert!(table.samples.iter().any(|s| s.id.name == "net.conns"));
        let c = conns(3.0);
        assert!(c.attributed < c.scanned, "not every socket is attributable");
    }

    /// The counters the real source would report, because it derives both from
    /// the rows it publishes: `scanned = rows.len()` with no cap or truncation
    /// (`sources/src/net/mod.rs`), and `attributed` is the count that resolved
    /// to a pid (`conns::attribute` returns exactly that). The fixture used to
    /// claim `scanned: 103` beside five rows — a source that does not exist,
    /// printed verbatim by the sources tile's footer (arc 16, D67 §4).
    #[test]
    fn conns_counters_are_what_the_real_source_would_report() {
        let c = conns(3.0);
        assert_eq!(c.scanned, c.rows.len(), "scanned is the row count");
        assert_eq!(
            c.attributed,
            c.rows.iter().filter(|r| r.pid.is_some()).count(),
            "attributed is the rows that resolved to a pid"
        );
    }

    /// The fold has to be reachable. The `conns` tier's connection band is at
    /// least 23 rows, so a fixture with five rows puts every row on the first
    /// page and no cursor can ever leave it — which is why D65 recorded the
    /// pty case driving the cursor past the fold as unwritable against the
    /// shipped fixture (arc 16).
    #[test]
    fn the_connection_table_is_longer_than_the_band_that_draws_it() {
        assert!(
            conns(3.0).rows.len() > 23,
            "the fixture must outrun the tallest band that can draw it"
        );
    }

    /// The first page has to show an unattributed socket. `attributed <
    /// scanned` is a number the tile draws, and a fixture that lists every
    /// browser socket first puts every unattributed one past the fold — where
    /// the page a reader actually sees cannot demonstrate it (arc 16; it broke
    /// `net`'s own `the_table_shows_states_rates_and_the_probe_strip`).
    #[test]
    fn the_first_page_of_connections_is_not_all_one_process() {
        let rows = conns(3.0).rows;
        let first = &rows[..8];
        assert!(
            first.iter().any(|r| r.pid.is_none()),
            "an unattributed socket must be on the first page"
        );
        let procs: std::collections::BTreeSet<&str> =
            first.iter().map(|r| r.process.as_str()).collect();
        assert!(
            procs.len() >= 4,
            "the first page is all the same thing: {procs:?}"
        );
    }

    /// A filtered tile and an unfiltered one have to differ by more than one
    /// row, or no test can tell the filter works (arc 16, D67 §4).
    #[test]
    fn there_are_enough_virtual_interfaces_for_the_filter_to_matter() {
        let all = [eno1_link(), wifi_link(true), bridge_link()]
            .into_iter()
            .chain(quiet_links())
            .collect::<Vec<_>>();
        let virt = all
            .iter()
            .filter(|l| matches!(l.kind, LinkKind::Virtual))
            .count();
        assert!(all.len() >= 6, "a workstation has more than three");
        assert!(virt >= 3, "the default filter must drop more than one row");
    }
}
