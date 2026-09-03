//! The net source (§5 cadence row, §8, brief arc 7 seam 2): one
//! `/proc/net/dev` read per tick for the rates, sysfs link attributes every
//! couple of seconds, the default route and the resolvers, the connection
//! table joined with a `/proc/*/fd` inode map at `Detail::Table` only, and
//! the latency probes behind `net-probe`. Everything it reads is a file the
//! user may read; nothing here needs a capability.

pub mod conns;
pub mod dev;
#[cfg(feature = "net-probe")]
pub mod probe;
pub mod route;

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gridwatch_store::keys::net::{self, Link, Probes};
use gridwatch_store::{
    Cadence, Datum, Detail, Label, Level, MetricId, Sample, Source, SourceCtx, SourceInfo,
    SourceState, SourceStatus, demo,
};

pub mod link;

/// `[sources.net]` (§9).
pub const OPTION_NAMES: &[&str] = &[
    "refresh_ms",
    "link_ms",
    "conns",
    "conns_ms",
    "probes",
    "probe_ms",
    "public_ip",
];
pub const MIN_REFRESH: Duration = Duration::from_millis(250);
pub const MAX_REFRESH: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub refresh: Duration,
    pub link: Duration,
    pub conns: bool,
    pub conns_every: Duration,
    /// Probe targets: `gateway` resolves to the default route's gateway.
    pub probes: Vec<String>,
    pub probe_every: Duration,
    /// Off by default: an outbound request is the user's decision (§9).
    pub public_ip: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            refresh: Duration::from_secs(1),
            link: Duration::from_secs(2),
            conns: true,
            conns_every: Duration::from_secs(2),
            probes: vec!["gateway".into(), "1.1.1.1".into()],
            probe_every: Duration::from_secs(1),
            public_ip: false,
        }
    }
}

fn clamp(d: Duration) -> Duration {
    d.clamp(MIN_REFRESH, MAX_REFRESH)
}

impl Options {
    pub fn from_table(t: &toml::Table) -> Options {
        let mut o = Options::default();
        let ms = |k: &str| {
            t.get(k)
                .and_then(|v| v.as_integer())
                .map(|n| Duration::from_millis(n.max(0) as u64))
        };
        if let Some(d) = ms("refresh_ms") {
            o.refresh = clamp(d);
            if o.refresh != d {
                tracing::warn!(
                    "[sources.net] refresh_ms = {} clamped to {}",
                    d.as_millis(),
                    o.refresh.as_millis()
                );
            }
        }
        if let Some(d) = ms("link_ms") {
            o.link = clamp(d);
        }
        if let Some(d) = ms("conns_ms") {
            o.conns_every = clamp(d);
        }
        if let Some(d) = ms("probe_ms") {
            o.probe_every = clamp(d);
        }
        if let Some(b) = t.get("conns").and_then(|v| v.as_bool()) {
            o.conns = b;
        }
        if let Some(b) = t.get("public_ip").and_then(|v| v.as_bool()) {
            o.public_ip = b;
        }
        if let Some(list) = t.get("probes").and_then(|v| v.as_array()) {
            o.probes = list
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        o
    }
}

/// `gridwatch doctor`'s rows (seam 8): what the source can read here. All
/// sysfs and procfs, so it is safe under `--offline`.
pub fn doctor() -> Vec<(gridwatch_store::Capability, bool, String)> {
    use gridwatch_store::Capability;
    let sys = PathBuf::from("/sys");
    let proc = PathBuf::from("/proc");
    let ifaces = link::interfaces(&sys);
    let rows = std::fs::read_to_string(proc.join("net/route"))
        .map(|t| route::parse(&t))
        .unwrap_or_default();
    let gw = route::default_route(&rows).map(|r| r.gateway.to_string());
    let mut out = vec![(
        Capability::Procfs,
        !ifaces.is_empty(),
        match gw {
            Some(gw) => format!("{} interfaces; default gateway {gw}", ifaces.len()),
            None => format!("{} interfaces; no default route", ifaces.len()),
        },
    )];
    #[cfg(feature = "net-probe")]
    {
        let ok = probe::icmp_available();
        out.push((
            Capability::PingSocket,
            ok,
            if ok {
                "unprivileged ICMP sockets allowed (net.ipv4.ping_group_range)".into()
            } else {
                "no unprivileged ICMP: the probes fall back to a TCP connect".to_string()
            },
        ));
    }
    out
}

fn named(key: &gridwatch_store::Key<f64>, label: &str) -> MetricId {
    MetricId {
        name: key.id.name,
        label: Label::Name(Arc::from(label)),
    }
}

/// The roots the sampler reads (the cpu source's pattern, so tests point at
/// a fixture tree).
#[derive(Clone, Debug)]
pub struct Roots {
    pub proc: PathBuf,
    pub sys: PathBuf,
    pub resolv: PathBuf,
}

impl Default for Roots {
    fn default() -> Roots {
        Roots {
            proc: PathBuf::from("/proc"),
            sys: PathBuf::from("/sys"),
            resolv: PathBuf::from("/etc/resolv.conf"),
        }
    }
}

/// One pass: counters → rates, and the slower reads when they are due.
pub struct Sampler {
    pub roots: Roots,
    pub options: Options,
    prev: HashMap<String, (Instant, dev::Counters)>,
    linked: Option<Instant>,
    conned: Option<Instant>,
    /// The last link Record per interface, so an unchanged one is not
    /// republished every two seconds.
    links: HashMap<String, Link>,
    /// The last route published, so a change is noticed and a repeat is not.
    route: Option<net::Route>,
    pub scan_ms: f64,
}

impl Sampler {
    pub fn new(roots: Roots, options: Options) -> Sampler {
        Sampler {
            roots,
            options,
            prev: HashMap::new(),
            linked: None,
            conned: None,
            links: HashMap::new(),
            route: None,
            scan_ms: 0.0,
        }
    }

    /// The rates for this tick.
    pub fn rates(&mut self, now: Instant) -> Vec<Sample> {
        let counters = dev::read(&self.roots.proc);
        let mut out = Vec::with_capacity(counters.len() * 4);
        for (name, c) in &counters {
            if let Some((then, prev)) = self.prev.get(name) {
                let r = c.rates(prev, now.saturating_duration_since(*then));
                for (key, v) in [
                    (&net::RX_BPS, r.rx_bps),
                    (&net::TX_BPS, r.tx_bps),
                    (&net::RX_PPS, r.rx_pps),
                    (&net::TX_PPS, r.tx_pps),
                    (&net::RX_DROP, r.rx_drop),
                    (&net::TX_DROP, r.tx_drop),
                    (&net::RX_ERR, r.rx_err),
                    (&net::TX_ERR, r.tx_err),
                ] {
                    out.push(Sample {
                        id: named(key, name),
                        datum: Datum::Scalar(v),
                    });
                }
            }
            self.prev.insert(name.clone(), (now, *c));
        }
        // An interface that went away stops being remembered.
        self.prev.retain(|k, _| counters.contains_key(k));
        out
    }

    /// The link Records, when they are due and when they changed.
    pub fn links(&mut self, now: Instant) -> Vec<Sample> {
        if self
            .linked
            .is_some_and(|t| now.saturating_duration_since(t) < self.options.link)
        {
            return Vec::new();
        }
        self.linked = Some(now);
        let mut out = Vec::new();
        let names = link::interfaces(&self.roots.sys);
        let addrs = link::inet6_addrs(&self.roots.proc);
        for name in &names {
            let l = link::read_link(
                &self.roots.sys,
                name,
                addrs.get(name).cloned().unwrap_or_default(),
            );
            if self.links.get(name) == Some(&l) {
                continue;
            }
            out.push(Sample {
                id: named(&net::SPEED_MBPS, name),
                datum: Datum::Scalar(l.speed_mbps as f64),
            });
            out.push(Sample {
                id: MetricId {
                    name: net::LINK.id.name,
                    label: Label::Name(Arc::from(name.as_str())),
                },
                datum: Datum::Record(Arc::new(l.clone())),
            });
            self.links.insert(name.clone(), l);
        }
        self.links.retain(|k, _| names.contains(k));
        out
    }

    /// The route Record: on the first tick, and whenever it **changes**.
    /// It used to send once and never again, so a VPN coming up or a cable
    /// swap left the tile naming the old interface for ever (arc 7a
    /// review, D57 amendment 20) — `links()` had always diffed; this now
    /// does the same.
    pub fn route(&mut self) -> Vec<Sample> {
        let r = route::read(&self.roots.proc, &self.roots.resolv, String::new());
        if self.route.as_ref() == Some(&r) {
            return Vec::new();
        }
        self.route = Some(r.clone());
        vec![Sample {
            id: net::ROUTE.id.clone(),
            datum: Datum::Record(Arc::new(r)),
        }]
    }

    /// The connection table, at `Detail::Table` and on its own cadence.
    pub fn conns(&mut self, now: Instant, detail: Detail) -> Vec<Sample> {
        if !self.options.conns || detail < Detail::Table {
            return Vec::new();
        }
        if self
            .conned
            .is_some_and(|t| now.saturating_duration_since(t) < self.options.conns_every)
        {
            return Vec::new();
        }
        self.conned = Some(now);
        let t0 = Instant::now();
        let mut rows = conns::read_all(&self.roots.proc);
        let map = conns::inode_map(&self.roots.proc);
        let attributed = conns::attribute(&mut rows, &map);
        self.scan_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let scanned = rows.len();
        vec![
            Sample {
                id: net::SCAN_MS.id.clone(),
                datum: Datum::Scalar((self.scan_ms * 10.0).round() / 10.0),
            },
            Sample {
                id: net::CONNS.id.clone(),
                datum: Datum::Record(Arc::new(gridwatch_store::keys::net::Conns {
                    rows,
                    scanned,
                    attributed,
                    scan_ms: (self.scan_ms * 10.0).round() / 10.0,
                })),
            },
        ]
    }

    /// The gateway a `gateway` probe target resolves to.
    pub fn gateway(&self) -> Option<IpAddr> {
        let rows = std::fs::read_to_string(self.roots.proc.join("net/route"))
            .map(|t| route::parse(&t))
            .unwrap_or_default();
        route::default_route(&rows).map(|r| IpAddr::V4(r.gateway))
    }
}

pub struct NetSource {
    options: Options,
}

impl NetSource {
    pub fn new(options: &toml::Table) -> NetSource {
        NetSource {
            options: Options::from_table(options),
        }
    }
}

fn status(cx: &SourceCtx, state: SourceState, reason: &str, hint: Option<&str>) {
    cx.status(SourceStatus {
        state,
        reason: Some(Arc::from(reason)),
        hint: hint.map(Arc::from),
        since: cx.clock.now(),
        last_sample: None,
        dropped: 0,
        restarts: cx.restarts,
    });
}

impl Source for NetSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: Cadence {
                hidden: Some(self.options.refresh.max(Duration::from_secs(2))),
                visible: self.options.refresh,
                focused: self.options.refresh,
                always_on: false,
            },
            ..demo::net_info()
        }
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let cadence = self.info().cadence;
        let mut sampler = Sampler::new(Roots::default(), self.options.clone());
        #[cfg(feature = "net-probe")]
        let mut prober = Prober::new(self.options.probes.clone());
        let mut last: Option<(SourceState, String)> = None;
        let mut set_status = |cx: &SourceCtx, state: SourceState, reason: &str| {
            let key = (state, reason.to_string());
            if last.as_ref() != Some(&key) {
                last = Some(key);
                status(cx, state, reason, None);
            }
        };
        set_status(&cx, SourceState::Starting, "starting");
        let mut first = true;
        loop {
            if !first {
                let level = cx.demand.level();
                let Some(period) = cadence.for_level(level) else {
                    if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                        return;
                    }
                    continue;
                };
                if !cx.sleep_until(cx.next_deadline(period)) {
                    return;
                }
            }
            first = false;
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
            if cx.demand.level() == Level::Paused {
                continue;
            }
            let now = Instant::now();
            let at = cx.clock.now();
            let mut samples = sampler.rates(now);
            samples.extend(sampler.links(now));
            samples.extend(sampler.route());
            samples.extend(sampler.conns(now, cx.demand.detail()));
            #[cfg(feature = "net-probe")]
            {
                let gw = sampler.gateway();
                samples.extend(prober.tick(now, gw, self.options.probe_every));
            }
            let ifaces = sampler.links.len().max(sampler.prev.len());
            if ifaces == 0 {
                set_status(
                    &cx,
                    SourceState::Unavailable,
                    "no interfaces in /proc/net/dev",
                );
            } else {
                set_status(&cx, SourceState::Ok, &format!("{ifaces} interfaces"));
            }
            if !samples.is_empty() {
                cx.emit(at, samples);
            }
        }
    }
}

/// One round the probe thread is asked for: the sequence number and the
/// targets, already resolved to addresses.
#[cfg(feature = "net-probe")]
type Round = (u16, Vec<(String, IpAddr)>);

/// One round's answer from the probe thread.
#[cfg(feature = "net-probe")]
struct Reply {
    target: String,
    addr: IpAddr,
    kind: gridwatch_store::keys::net::ProbeKind,
    ms: Option<f64>,
}

/// The probes' own state: a ring per target, on its own cadence, with the
/// **blocking** work on a helper thread. Inline, two silent targets held
/// the source thread for 1.8 s a tick and delayed every rate sample with
/// it (arc 7a review, D57 amendment 22); the brief always said helper
/// thread. The thread lives as long as the `Prober`: dropping it closes
/// the request channel and the thread returns.
#[cfg(feature = "net-probe")]
pub struct Prober {
    targets: Vec<String>,
    rings: HashMap<String, probe::Ring>,
    last: Option<Instant>,
    seq: u16,
    degraded: Option<String>,
    /// A round in flight: the thread is probing and has not answered yet.
    pending: bool,
    ask: Option<std::sync::mpsc::Sender<Round>>,
    replies: Option<std::sync::mpsc::Receiver<Vec<Reply>>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "net-probe")]
impl Drop for Prober {
    fn drop(&mut self) {
        // Close the channel first, then wait: the thread is at most one
        // round (a bounded number of timeouts) from noticing.
        self.ask = None;
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[cfg(feature = "net-probe")]
impl Prober {
    pub fn new(targets: Vec<String>) -> Prober {
        let icmp = probe::icmp_available();
        let kind = if icmp {
            gridwatch_store::keys::net::ProbeKind::Icmp
        } else {
            gridwatch_store::keys::net::ProbeKind::Tcp
        };
        let (ask, asked) = std::sync::mpsc::channel::<Round>();
        let (answer, replies) = std::sync::mpsc::channel::<Vec<Reply>>();
        let worker = std::thread::Builder::new()
            .name("gw-net-probe".into())
            .spawn(move || {
                while let Ok((seq, round)) = asked.recv() {
                    let mut out = Vec::with_capacity(round.len());
                    for (target, addr) in round {
                        // ICMP here is v4 only, so a v6 target measures
                        // with a TCP connect rather than reporting a loss
                        // for a path nobody probed (review B2).
                        let kind = if kind == gridwatch_store::keys::net::ProbeKind::Icmp
                            && addr.is_ipv4()
                        {
                            gridwatch_store::keys::net::ProbeKind::Icmp
                        } else {
                            gridwatch_store::keys::net::ProbeKind::Tcp
                        };
                        let ms = match kind {
                            gridwatch_store::keys::net::ProbeKind::Icmp => {
                                probe::icmp_probe(addr, seq, probe::TIMEOUT)
                            }
                            gridwatch_store::keys::net::ProbeKind::Tcp => {
                                probe::tcp_probe(addr, probe::TCP_PORT, probe::TIMEOUT)
                            }
                        };
                        out.push(Reply {
                            target,
                            addr,
                            kind,
                            ms,
                        });
                    }
                    if answer.send(out).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Prober {
            targets,
            rings: HashMap::new(),
            last: None,
            seq: 0,
            degraded: (!icmp)
                .then(|| "no unprivileged ICMP socket; measuring with a TCP connect".to_string()),
            pending: false,
            ask: Some(ask),
            replies: Some(replies),
            worker,
        }
    }

    /// Send a round if one is due, and turn whatever the thread has
    /// finished into samples. Never blocks.
    pub fn tick(&mut self, now: Instant, gateway: Option<IpAddr>, every: Duration) -> Vec<Sample> {
        if self.targets.is_empty() {
            return Vec::new();
        }
        let due = self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) >= every);
        if due && !self.pending {
            let round: Vec<(String, IpAddr)> = self
                .targets
                .iter()
                .filter_map(|t| {
                    let addr = if t == "gateway" {
                        gateway
                    } else {
                        t.parse::<IpAddr>().ok()
                    };
                    addr.map(|a| (t.clone(), a))
                })
                .collect();
            if !round.is_empty()
                && let Some(ask) = self.ask.as_ref()
            {
                self.seq = self.seq.wrapping_add(1);
                if ask.send((self.seq, round)).is_ok() {
                    self.pending = true;
                    self.last = Some(now);
                }
            }
        }
        let Some(replies) = self.replies.as_ref() else {
            return Vec::new();
        };
        let Ok(round) = replies.try_recv() else {
            return Vec::new();
        };
        self.pending = false;
        let mut stats = Vec::with_capacity(round.len());
        let mut out = Vec::new();
        for Reply {
            target,
            addr,
            kind,
            ms,
        } in round
        {
            let target = &target;
            let ring = self.rings.entry(target.clone()).or_default();
            ring.push(ms);
            if let Some(ms) = ms {
                out.push(Sample {
                    id: named(&net::RTT_MS, target),
                    datum: Datum::Scalar((ms * 100.0).round() / 100.0),
                });
            }
            let stat = ring.stat(target, &addr.to_string(), kind);
            out.push(Sample {
                id: named(&net::LOSS_PCT, target),
                datum: Datum::Scalar(stat.loss_pct),
            });
            stats.push(stat);
        }
        if !stats.is_empty() {
            out.push(Sample {
                id: net::PROBE.id.clone(),
                datum: Datum::Record(Arc::new(Probes {
                    targets: stats,
                    degraded: self.degraded.clone(),
                })),
            });
        }
        out
    }
}

pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(NetSource::new(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_roots() -> Roots {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net");
        Roots {
            proc: base.join("proc"),
            sys: base.join("sys"),
            resolv: base.join("proc/resolv.conf"),
        }
    }

    /// The probes run on a helper thread: a round against an address that
    /// will never answer must not hold the caller for the timeout, and the
    /// thread must end when the `Prober` is dropped (arc 7a review).
    #[cfg(feature = "net-probe")]
    #[test]
    fn probes_do_not_block_the_source_thread() {
        // TEST-NET-1 (RFC 5737): documentation space, routed nowhere.
        let mut p = Prober::new(vec!["192.0.2.1".to_string()]);
        let t0 = Instant::now();
        let first = p.tick(Instant::now(), None, Duration::from_millis(0));
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "the first tick blocked for {elapsed:?} — the probe is inline again"
        );
        assert!(first.is_empty(), "the answer cannot be ready yet");
        // Ticking again while a round is in flight is also instant, and
        // does not start a second round.
        let t0 = Instant::now();
        let _ = p.tick(Instant::now(), None, Duration::from_millis(0));
        assert!(t0.elapsed() < Duration::from_millis(100));
        // Dropping joins the thread; if it leaked, this test would hang
        // rather than finish.
        drop(p);
    }

    #[test]
    fn options_parse_and_clamp() {
        let t: toml::Table = toml::from_str(
            r#"refresh_ms = 10
link_ms = 60000
conns = false
probes = ["gateway"]
public_ip = true"#,
        )
        .unwrap();
        let o = Options::from_table(&t);
        assert_eq!(o.refresh, MIN_REFRESH);
        assert_eq!(o.link, MAX_REFRESH);
        assert!(!o.conns);
        assert_eq!(o.probes, ["gateway"]);
        assert!(o.public_ip, "opt-in, but honoured when asked for");
        assert_eq!(Options::from_table(&toml::Table::new()), Options::default());
        assert!(!Options::default().public_ip, "off by default (§9)");
        assert_eq!(OPTION_NAMES.len(), 7);
    }

    #[test]
    fn the_first_tick_has_no_rates_and_the_second_does() {
        let mut s = Sampler::new(fixture_roots(), Options::default());
        let t0 = Instant::now();
        let first = s.rates(t0);
        assert!(first.is_empty(), "a rate needs two samples");
        let second = s.rates(t0 + Duration::from_secs(1));
        assert!(!second.is_empty());
        // The fixture is a still file, so every rate is zero — and that is
        // the honest answer, not a missing sample.
        assert!(
            second
                .iter()
                .all(|x| matches!(x.datum, Datum::Scalar(v) if v == 0.0))
        );
        let names: Vec<&str> = second.iter().map(|x| x.id.name).collect();
        assert!(names.contains(&"net.rx_bps") && names.contains(&"net.tx_drop"));
    }

    #[test]
    fn links_are_published_once_until_they_change() {
        let mut s = Sampler::new(fixture_roots(), Options::default());
        let t0 = Instant::now();
        let first = s.links(t0);
        assert!(
            first.iter().filter(|x| x.id.name == "net.link").count() >= 3,
            "every interface in the fixture"
        );
        // Too soon, and then unchanged: nothing.
        assert!(s.links(t0 + Duration::from_millis(500)).is_empty());
        assert!(s.links(t0 + Duration::from_secs(5)).is_empty());
        // The route Record goes out once.
        assert_eq!(s.route().len(), 1);
        assert!(s.route().is_empty());
    }

    #[test]
    fn the_connection_table_is_table_detail_work() {
        let mut s = Sampler::new(Roots::default(), Options::default());
        let t0 = Instant::now();
        assert!(s.conns(t0, Detail::Meters).is_empty(), "not at meters");
        let out = s.conns(t0, Detail::Table);
        assert_eq!(out.len(), 2, "the scan's cost and the table");
        assert!(s.conns(t0, Detail::Table).is_empty(), "on its own cadence");
        let Datum::Record(r) = &out[1].datum else {
            panic!()
        };
        let c = r
            .as_any()
            .downcast_ref::<gridwatch_store::keys::net::Conns>()
            .unwrap();
        assert!(c.scanned > 0, "this machine has sockets");
        assert!(c.attributed <= c.scanned);
        assert!(s.scan_ms > 0.0 && s.scan_ms < 5_000.0, "{} ms", s.scan_ms);
        // Turned off, it costs nothing.
        let mut off = Sampler::new(
            Roots::default(),
            Options {
                conns: false,
                ..Options::default()
            },
        );
        assert!(off.conns(t0, Detail::Table).is_empty());
    }
}
