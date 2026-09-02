//! Network keys (§8, brief arc 7 seam 1): per-interface rates and link
//! state, the default route with DNS, the connection table and the latency
//! probes. Everything is labelled by interface name or probe target, so a
//! tile filters by glob without the store knowing what an interface is.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit};
use crate::source::SourceId;

pub const SOURCE: SourceId = SourceId("net");

/// Bytes and packets per second, per interface.
pub const RX_BPS: Key<f64> = Key::new("net.rx_bps");
pub const TX_BPS: Key<f64> = Key::new("net.tx_bps");
pub const RX_PPS: Key<f64> = Key::new("net.rx_pps");
pub const TX_PPS: Key<f64> = Key::new("net.tx_pps");
/// Drops and errors per second (the counters' deltas).
pub const RX_DROP: Key<f64> = Key::new("net.rx_drop");
pub const TX_DROP: Key<f64> = Key::new("net.tx_drop");
pub const RX_ERR: Key<f64> = Key::new("net.rx_err");
pub const TX_ERR: Key<f64> = Key::new("net.tx_err");
/// Negotiated link speed; `-1` when the driver cannot say (a carrier-less
/// bridge, a down Wi-Fi NIC — never guess).
pub const SPEED_MBPS: Key<f64> = Key::new("net.speed_mbps");
pub const LINK: Key<Link> = Key::new("net.link");
pub const ROUTE: Key<Route> = Key::new("net.route");
pub const CONNS: Key<Conns> = Key::new("net.conns");
pub const PROBE: Key<Probes> = Key::new("net.probe");
/// Round-trip time and loss per probe target.
pub const RTT_MS: Key<f64> = Key::new("net.rtt_ms");
pub const LOSS_PCT: Key<f64> = Key::new("net.loss_pct");
/// The connection scan's wall ms (the `sources` tile's note).
pub const SCAN_MS: Key<f64> = Key::new("net.scan_ms");

/// A driver that cannot report its speed says this.
pub const SPEED_UNKNOWN: f64 = -1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Ether,
    Wifi,
    Loopback,
    /// A bridge, a veth, a docker interface — the noise a tile hides.
    Virtual,
    Tun,
    #[default]
    Other,
}

impl LinkKind {
    pub fn name(self) -> &'static str {
        match self {
            LinkKind::Ether => "ether",
            LinkKind::Wifi => "wifi",
            LinkKind::Loopback => "loopback",
            LinkKind::Virtual => "virtual",
            LinkKind::Tun => "tun",
            LinkKind::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WifiInfo {
    pub ssid: String,
    pub signal_dbm: i32,
    pub freq_mhz: u32,
    pub rx_bitrate_kbps: u32,
    pub tx_bitrate_kbps: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub iface: String,
    pub up: bool,
    pub carrier: bool,
    /// `up`, `down`, `unknown` — the kernel's own word.
    pub operstate: String,
    pub mtu: u32,
    pub mac: String,
    pub kind: LinkKind,
    /// `-1` when unknown.
    pub speed_mbps: i64,
    pub carrier_changes: u64,
    pub addrs: Vec<String>,
    pub wifi: Option<WifiInfo>,
}

impl Link {
    /// What a tile shows as the link's one-word state.
    pub fn state(&self) -> &'static str {
        if self.up && self.carrier {
            "up"
        } else if self.up {
            "no carrier"
        } else {
            "down"
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub default_iface: String,
    pub gateway: String,
    pub prefsrc: String,
    pub dns: Vec<String>,
    /// Only when `[sources.net] public_ip = true` — an outbound request is
    /// the user's decision (§9).
    pub public_ip: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Proto {
    #[default]
    Tcp,
    Tcp6,
    Udp,
    Udp6,
}

impl Proto {
    pub fn name(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Tcp6 => "tcp6",
            Proto::Udp => "udp",
            Proto::Udp6 => "udp6",
        }
    }

    pub fn is_udp(self) -> bool {
        matches!(self, Proto::Udp | Proto::Udp6)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conn {
    pub proto: Proto,
    pub local: String,
    pub remote: String,
    /// `ESTAB`, `LISTEN`, … (the kernel's `st` column, named).
    pub state: String,
    pub inode: u64,
    /// `None` when `/proc/<pid>/fd` was not readable — the uid still is.
    pub pid: Option<u32>,
    pub uid: u32,
    pub process: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conns {
    pub rows: Vec<Conn>,
    /// Sockets seen, and how many got a pid: the honest denominator.
    pub scanned: usize,
    pub attributed: usize,
    pub scan_ms: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeKind {
    #[default]
    Icmp,
    /// The fallback when an ICMP socket is refused.
    Tcp,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbeStat {
    pub target: String,
    pub addr: String,
    pub kind: ProbeKind,
    pub min_ms: f64,
    pub avg_ms: f64,
    pub max_ms: f64,
    /// Mean deviation, as `ping` prints it.
    pub mdev_ms: f64,
    /// RFC 3550's interarrival jitter.
    pub jitter_ms: f64,
    pub loss_pct: f64,
    pub sent: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Probes {
    pub targets: Vec<ProbeStat>,
    /// Set when the ICMP socket was refused and the probes fell back.
    pub degraded: Option<String>,
}

fn decode<T: for<'de> Deserialize<'de> + RecordValue>(
    v: serde_json::Value,
) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<T>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

fn decode_link(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Link>(v)
}

fn decode_route(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Route>(v)
}

fn decode_conns(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Conns>(v)
}

fn decode_probes(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Probes>(v)
}

macro_rules! scalar {
    ($name:expr, $unit:ident, $doc:expr) => {
        KeyMeta {
            name: $name,
            unit: Unit::$unit,
            kind: DatumKind::Scalar,
            source: SOURCE,
            doc: $doc,
            decode: None,
        }
    };
}

pub static METAS: &[KeyMeta] = &[
    scalar!(
        "net.rx_bps",
        BytesPerSec,
        "receive rate per {iface}, from /proc/net/dev deltas over the measured interval"
    ),
    scalar!("net.tx_bps", BytesPerSec, "transmit rate per {iface}"),
    scalar!(
        "net.rx_pps",
        Count,
        "receive packets per second per {iface}"
    ),
    scalar!(
        "net.tx_pps",
        Count,
        "transmit packets per second per {iface}"
    ),
    scalar!("net.rx_drop", Count, "receive drops per second per {iface}"),
    scalar!(
        "net.tx_drop",
        Count,
        "transmit drops per second per {iface}"
    ),
    scalar!("net.rx_err", Count, "receive errors per second per {iface}"),
    scalar!(
        "net.tx_err",
        Count,
        "transmit errors per second per {iface}"
    ),
    scalar!(
        "net.speed_mbps",
        Count,
        "negotiated link speed per {iface} in Mb/s; -1 when the driver cannot say (never guessed)"
    ),
    scalar!(
        "net.rtt_ms",
        Milliseconds,
        "round-trip time per probe {target} (unprivileged ICMP, or a TCP connect)"
    ),
    scalar!(
        "net.loss_pct",
        Percent,
        "loss per probe {target} over its ring"
    ),
    scalar!(
        "net.scan_ms",
        Milliseconds,
        "wall ms of the last /proc/*/fd socket scan (the sources tile's note)"
    ),
    KeyMeta {
        name: "net.link",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "link state per {iface}: up/carrier/operstate, mtu, mac, kind, speed, carrier flaps, addresses and the Wi-Fi details when it is a radio",
        decode: Some(decode_link),
    },
    KeyMeta {
        name: "net.route",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the default route (interface, gateway, source address), the DNS servers, and the public IP when the user opted in",
        decode: Some(decode_route),
    },
    KeyMeta {
        name: "net.conns",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the connection table at Detail::Table: protocol, endpoints, state, and the owning process where /proc/<pid>/fd was readable (the uid otherwise)",
        decode: Some(decode_conns),
    },
    KeyMeta {
        name: "net.probe",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "latency statistics per target over a 60-sample ring: min/avg/max/mdev, RFC 3550 jitter and loss",
        decode: Some(decode_probes),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_states_read_like_the_kernel() {
        let mut l = Link {
            up: true,
            carrier: true,
            ..Link::default()
        };
        assert_eq!(l.state(), "up");
        l.carrier = false;
        assert_eq!(l.state(), "no carrier");
        l.up = false;
        assert_eq!(l.state(), "down");
        assert_eq!(LinkKind::Wifi.name(), "wifi");
        assert!(Proto::Udp6.is_udp());
        assert!(!Proto::Tcp.is_udp());
        assert_eq!(Proto::Tcp6.name(), "tcp6");
    }
}
