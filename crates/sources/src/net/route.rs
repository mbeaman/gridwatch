//! The default route and the resolvers (brief arc 7 seam 2).
//! `/proc/net/route`'s addresses are **little-endian hex** (`0164A8C0` is
//! 192.168.100.1) and the default row is `destination == 0` with
//! `RTF_GATEWAY`; the lowest metric wins. `/etc/resolv.conf` on torch is
//! systemd-resolved's stub (127.0.0.53), so that is labelled rather than
//! reported as a real server.

use std::net::Ipv4Addr;
use std::path::Path;

use gridwatch_store::keys::net::Route;

/// `RTF_UP | RTF_GATEWAY` from `route.h`.
const RTF_UP: u16 = 0x1;
const RTF_GATEWAY: u16 = 0x2;
/// What systemd-resolved's stub answers on.
pub const RESOLVED_STUB: &str = "127.0.0.53";

/// One row of `/proc/net/route`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteRow {
    pub iface: String,
    pub destination: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub flags: u16,
    pub metric: u32,
}

/// The kernel writes each address as little-endian hex.
fn hex_le_ip(s: &str) -> Option<Ipv4Addr> {
    let n = u32::from_str_radix(s, 16).ok()?;
    let b = n.to_le_bytes();
    Some(Ipv4Addr::new(b[0], b[1], b[2], b[3]))
}

pub fn parse(text: &str) -> Vec<RouteRow> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            continue;
        }
        let (Some(destination), Some(gateway), Some(mask)) =
            (hex_le_ip(f[1]), hex_le_ip(f[2]), hex_le_ip(f[7]))
        else {
            continue;
        };
        out.push(RouteRow {
            iface: f[0].to_string(),
            destination,
            gateway,
            mask,
            // `fib_route_seq_show` prints the flags as `%04X`. Parsing
            // them as decimal happened to work for every value torch
            // emits, and would silently drop any row whose flags carry a
            // hex letter (arc 7a review, D57 amendment 23).
            flags: u16::from_str_radix(f[3], 16).unwrap_or(0),
            metric: f[6].parse().unwrap_or(0),
        });
    }
    out
}

/// The default route: `0.0.0.0` with a gateway, lowest metric first.
pub fn default_route(rows: &[RouteRow]) -> Option<&RouteRow> {
    rows.iter()
        .filter(|r| {
            r.destination == Ipv4Addr::UNSPECIFIED
                && r.flags & RTF_GATEWAY != 0
                && r.flags & RTF_UP != 0
        })
        .min_by_key(|r| r.metric)
}

/// The resolvers `/etc/resolv.conf` names. The stub is reported as itself
/// with a note; the caller may replace it with resolved's real list.
pub fn resolv_conf(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let l = l.split('#').next().unwrap_or("").trim();
            l.strip_prefix("nameserver").map(|r| r.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// True when the only resolver is systemd-resolved's stub.
pub fn is_stub(dns: &[String]) -> bool {
    dns.len() == 1 && dns[0] == RESOLVED_STUB
}

/// Build the Record from a proc root and a resolv.conf path.
pub fn read(proc: &Path, resolv: &Path, prefsrc: String) -> Route {
    let rows = std::fs::read_to_string(proc.join("net/route"))
        .map(|t| parse(&t))
        .unwrap_or_default();
    let default = default_route(&rows);
    let mut dns = resolv_conf(resolv);
    if is_stub(&dns) {
        dns = vec![format!("{RESOLVED_STUB} (systemd-resolved)")];
    }
    Route {
        default_iface: default.map(|r| r.iface.clone()).unwrap_or_default(),
        gateway: default.map(|r| r.gateway.to_string()).unwrap_or_default(),
        prefsrc,
        dns,
        public_ip: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net/proc/net/route");
        std::fs::read_to_string(p).expect("the fixture reads")
    }

    #[test]
    fn torchs_default_route_decodes_little_endian() {
        let rows = parse(&fixture());
        assert!(!rows.is_empty());
        let d = default_route(&rows).expect("a default route");
        assert_eq!(d.iface, "eno1");
        assert_eq!(
            d.gateway,
            Ipv4Addr::new(192, 168, 100, 1),
            "0164A8C0 is little-endian"
        );
        assert_eq!(d.destination, Ipv4Addr::UNSPECIFIED);
        // Every row on this machine parses.
        assert!(rows.iter().all(|r| r.iface.len() > 1));
    }

    #[test]
    fn the_lowest_metric_wins_and_junk_is_skipped() {
        let text = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                    eth0\t00000000\t0102030A\t0003\t0\t0\t600\t00000000\n\
                    wlan0\t00000000\t0102030B\t0003\t0\t0\t100\t00000000\n\
                    eth0\t0002000A\t00000000\t0001\t0\t0\t0\t00FFFFFF\n";
        let rows = parse(text);
        assert_eq!(rows.len(), 3);
        let d = default_route(&rows).unwrap();
        assert_eq!(d.iface, "wlan0", "metric 100 beats 600");
        // A route with no gateway flag is not a default route.
        let only_local = parse("h\neth0\t0002000A\t00000000\t0001\t0\t0\t0\t00FFFFFF\n");
        assert!(default_route(&only_local).is_none());
        assert!(parse("").is_empty());
        assert!(parse("header only\n").is_empty());
        assert!(parse("h\nshort line\n").is_empty());
    }

    #[test]
    fn resolvers_and_the_stub() {
        let dir = std::env::temp_dir().join(format!("gw-resolv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("resolv.conf");
        std::fs::write(&p, "# comment\nnameserver 127.0.0.53\noptions edns0\n").unwrap();
        let dns = resolv_conf(&p);
        assert_eq!(dns, ["127.0.0.53"]);
        assert!(is_stub(&dns));
        std::fs::write(&p, "nameserver 1.1.1.1\nnameserver 9.9.9.9\n").unwrap();
        let dns = resolv_conf(&p);
        assert_eq!(dns.len(), 2);
        assert!(!is_stub(&dns));
        assert!(resolv_conf(Path::new("/nonexistent")).is_empty());
        // The Record labels the stub for what it is.
        std::fs::write(&p, "nameserver 127.0.0.53\n").unwrap();
        let r = read(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net/proc"),
            &p,
            "192.168.100.154".into(),
        );
        assert!(r.dns[0].contains("systemd-resolved"), "{:?}", r.dns);
        assert_eq!(r.default_iface, "eno1", "the fixture's own route");
        assert_eq!(r.gateway, "192.168.100.1");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
