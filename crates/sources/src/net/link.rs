//! Link state from sysfs (brief arc 7 seam 2): `/sys/class/net/<if>/…`
//! read every couple of seconds, not every tick. `speed` is `-1` on a
//! carrier-less bridge and unreadable (`EINVAL`) on a down Wi-Fi NIC — both
//! are *unknown*, never a number; the presence of `wireless`/`phy80211` is
//! how a radio is recognised.

use std::path::Path;

use gridwatch_store::keys::net::{Link, LinkKind, SPEED_UNKNOWN};

/// `ARPHRD_ETHER` and `ARPHRD_LOOPBACK` from `if_arp.h`.
const ARPHRD_ETHER: u32 = 1;
const ARPHRD_LOOPBACK: u32 = 772;
/// `IFF_UP` and `IFF_RUNNING`.
const IFF_UP: u64 = 0x1;

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_num<T: std::str::FromStr>(p: &Path) -> Option<T> {
    read_trim(p)?.parse().ok()
}

/// The interface names sysfs knows about, sorted.
pub fn interfaces(sys: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(sys.join("class/net"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// What kind of interface this is, from sysfs alone.
pub fn kind_of(dir: &Path, name: &str) -> LinkKind {
    if dir.join("wireless").exists() || dir.join("phy80211").exists() {
        return LinkKind::Wifi;
    }
    match read_num::<u32>(&dir.join("type")) {
        Some(ARPHRD_LOOPBACK) => LinkKind::Loopback,
        Some(ARPHRD_ETHER) => {
            // A bridge, veth or docker interface is an Ethernet type too;
            // sysfs tells them apart by what hangs off the directory.
            if dir.join("bridge").exists()
                || dir.join("brport").exists()
                || name.starts_with("veth")
                || name.starts_with("docker")
                || name.starts_with("virbr")
                || name.starts_with("br-")
            {
                LinkKind::Virtual
            } else {
                LinkKind::Ether
            }
        }
        _ if name.starts_with("tun") || name.starts_with("tap") || name.starts_with("wg") => {
            LinkKind::Tun
        }
        _ => LinkKind::Other,
    }
}

/// Read one interface's link state. Addresses come from the caller (one
/// `getifaddrs` serves every interface).
pub fn read_link(sys: &Path, name: &str, addrs: Vec<String>) -> Link {
    let dir = sys.join("class/net").join(name);
    let operstate = read_trim(&dir.join("operstate")).unwrap_or_else(|| "unknown".into());
    let carrier = read_num::<u8>(&dir.join("carrier")).unwrap_or(0) == 1;
    let flags = read_trim(&dir.join("flags"))
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    // `speed` is only meaningful with a carrier: a down radio's read fails
    // outright, and a carrier-less bridge answers a number it negotiated
    // with nobody (torch's answers `10000`). Both are unknown.
    let speed = read_num::<i64>(&dir.join("speed"))
        .filter(|s| *s > 0 && carrier)
        .unwrap_or(SPEED_UNKNOWN as i64);
    Link {
        up: flags & IFF_UP != 0 || operstate == "up",
        carrier,
        operstate,
        mtu: read_num(&dir.join("mtu")).unwrap_or(0),
        mac: read_trim(&dir.join("address")).unwrap_or_default(),
        kind: kind_of(&dir, name),
        speed_mbps: speed,
        carrier_changes: read_num(&dir.join("carrier_changes")).unwrap_or(0),
        addrs,
        wifi: None,
        iface: name.to_string(),
    }
}

/// The project's one glob rule (`store::rules::glob`, D57 amendment 9):
/// `*` anywhere, so `en*1` filters the way the person writing it meant.
pub use gridwatch_store::rules::glob as matches;

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net/sys")
    }

    #[test]
    fn reads_torchs_links_with_unknown_speeds_honest() {
        let names = interfaces(&sys());
        assert!(names.contains(&"eno1".to_string()));
        assert!(names.contains(&"wlp7s0".to_string()));
        let eno1 = read_link(&sys(), "eno1", vec!["192.168.100.154/24".into()]);
        assert!(eno1.up && eno1.carrier);
        assert_eq!(eno1.operstate, "up");
        assert_eq!(eno1.speed_mbps, 1000, "1 GbE negotiated, not the NIC's 2.5");
        assert_eq!(eno1.mtu, 1500);
        assert_eq!(eno1.kind, LinkKind::Ether);
        assert_eq!(eno1.state(), "up");
        assert_eq!(eno1.addrs.len(), 1);
        let wifi = read_link(&sys(), "wlp7s0", Vec::new());
        assert_eq!(wifi.kind, LinkKind::Wifi, "the wireless directory says so");
        assert!(!wifi.carrier);
        assert_eq!(
            wifi.speed_mbps, -1,
            "the down radio's speed is unreadable, not a number"
        );
        assert_eq!(wifi.state(), "down");
        let lo = read_link(&sys(), "lo", Vec::new());
        assert_eq!(lo.kind, LinkKind::Loopback);
        let br = interfaces(&sys())
            .into_iter()
            .find(|n| n.starts_with("br-"))
            .expect("the bridge fixture");
        let bridge = read_link(&sys(), &br, Vec::new());
        assert_eq!(bridge.kind, LinkKind::Virtual);
        // This bridge does have a carrier, and the kernel answers 10000 for
        // a virtual link — that is what it says, so that is what is shown.
        assert_eq!(bridge.speed_mbps, 10_000);
        assert!(bridge.carrier);
        // A carrier-less link's speed is unknown whatever the file holds.
        let dir = std::env::temp_dir().join(format!("gw-link-{}", std::process::id()));
        let iface = dir.join("class/net/br0");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&iface).unwrap();
        std::fs::write(iface.join("speed"), "10000\n").unwrap();
        std::fs::write(iface.join("carrier"), "0\n").unwrap();
        std::fs::write(iface.join("operstate"), "down\n").unwrap();
        std::fs::write(iface.join("type"), "1\n").unwrap();
        let no_carrier = read_link(&dir, "br0", Vec::new());
        assert_eq!(
            no_carrier.speed_mbps, -1,
            "a speed negotiated with nobody is not a speed"
        );
        assert_eq!(no_carrier.state(), "down");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn globs_and_missing_trees() {
        assert!(matches("*", "anything"));
        assert!(matches("en*", "eno1"));
        assert!(matches("*1", "eno1"), "a trailing-star suffix");
        assert!(!matches("*0", "eno1"), "eno1 does not end in 0");
        assert!(matches("*7s0", "wlp7s0"));
        assert!(!matches("en", "eno1"));
        assert!(matches("*veth*", "x-veth-y"));
        // A star in the middle: the private copy this now delegates to
        // `store::rules::glob` for got this wrong.
        assert!(matches("en*1", "eno1"));
        assert!(!matches("en*1", "eno2"));
        assert!(matches("br-*", "br-6bb7413a559e"));
        assert!(interfaces(Path::new("/nonexistent")).is_empty());
        let missing = read_link(Path::new("/nonexistent"), "eth9", Vec::new());
        assert_eq!(missing.speed_mbps, -1);
        assert!(!missing.up);
        assert_eq!(missing.operstate, "unknown");
    }
}

/// The IPv6 addresses the kernel lists in `/proc/net/if_inet6`, by
/// interface. Format: 32 hex digits, ifindex, prefix length, scope, flags,
/// name — all whitespace separated.
///
/// IPv4 addresses are **not** in procfs: `ip addr` reads them over
/// netlink, and `getifaddrs` needs `unsafe`, which every crate here
/// forbids. So `Link.addrs` carries the v6 addresses and the docs say so
/// (arc 7a review, D57 amendment 24); the v4 path is owed and needs a
/// decision about netlink.
pub fn inet6_addrs(proc: &Path) -> std::collections::HashMap<String, Vec<String>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = Default::default();
    let Ok(text) = std::fs::read_to_string(proc.join("net/if_inet6")) else {
        return out;
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 6 || f[0].len() != 32 {
            continue;
        }
        let mut bytes = [0u8; 16];
        let mut ok = true;
        for (i, b) in bytes.iter_mut().enumerate() {
            match u8::from_str_radix(&f[0][i * 2..i * 2 + 2], 16) {
                Ok(v) => *b = v,
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let prefix = u32::from_str_radix(f[2], 16).unwrap_or(0);
        let addr = std::net::Ipv6Addr::from(bytes);
        out.entry(f[5].to_string())
            .or_default()
            .push(format!("{addr}/{prefix}"));
    }
    out
}

#[cfg(test)]
mod inet6_tests {
    use super::*;

    #[test]
    fn the_v6_addresses_come_out_by_interface() {
        let proc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net/proc");
        let by_iface = inet6_addrs(&proc);
        assert!(!by_iface.is_empty(), "torch's fixture lists addresses");
        let lo = by_iface.get("lo").expect("loopback is always there");
        assert_eq!(lo, &["::1/128".to_string()]);
        let eno1 = by_iface.get("eno1").expect("the wired interface");
        assert!(
            eno1.iter()
                .all(|a| a.starts_with("fe80:") && a.ends_with("/64")),
            "link-local with its prefix: {eno1:?}"
        );
        // A tree without the file yields nothing rather than panicking.
        assert!(inet6_addrs(Path::new("/nonexistent")).is_empty());
    }
}
