//! The connection table (brief arc 7 seam 2): `/proc/net/{tcp,tcp6,udp,
//! udp6}` parsed in-tree, joined with an inode → pid map built from
//! `/proc/*/fd`. Addresses are **little-endian hex** for IPv4 and four
//! little-endian 32-bit words for IPv6; ports are big-endian hex; `st` is
//! the TCP state table. The scan reads what it may: on torch 143 of 628
//! processes are readable as the user, and a socket whose owner is not
//! shows its uid instead of a lie.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use gridwatch_store::keys::net::{Conn, Proto};

/// The kernel's TCP state names, indexed by the `st` column.
pub fn state_name(st: u8) -> &'static str {
    match st {
        0x01 => "ESTAB",
        0x02 => "SYN-SENT",
        0x03 => "SYN-RECV",
        0x04 => "FIN-WAIT1",
        0x05 => "FIN-WAIT2",
        0x06 => "TIME-WAIT",
        0x07 => "CLOSE",
        0x08 => "CLOSE-WAIT",
        0x09 => "LAST-ACK",
        0x0A => "LISTEN",
        0x0B => "CLOSING",
        _ => "UNKNOWN",
    }
}

/// `0164A8C0:0016` → `192.168.100.1:22`.
pub fn parse_addr(s: &str, v6: bool) -> Option<(IpAddr, u16)> {
    let (addr, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    if v6 {
        if addr.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        // Four 32-bit words, each little-endian.
        for w in 0..4 {
            let word = u32::from_str_radix(&addr[w * 8..w * 8 + 8], 16).ok()?;
            bytes[w * 4..w * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        Some((IpAddr::V6(Ipv6Addr::from(bytes)), port))
    } else {
        let n = u32::from_str_radix(addr, 16).ok()?;
        let b = n.to_le_bytes();
        Some((IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])), port))
    }
}

fn show(addr: IpAddr, port: u16) -> String {
    match addr {
        IpAddr::V4(a) => format!("{a}:{port}"),
        IpAddr::V6(a) => format!("[{a}]:{port}"),
    }
}

/// Parse one of the four tables.
pub fn parse(text: &str, proto: Proto) -> Vec<Conn> {
    let v6 = matches!(proto, Proto::Tcp6 | Proto::Udp6);
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 {
            continue;
        }
        let (Some((la, lp)), Some((ra, rp))) = (parse_addr(f[1], v6), parse_addr(f[2], v6)) else {
            continue;
        };
        let st = u8::from_str_radix(f[3], 16).unwrap_or(0);
        out.push(Conn {
            proto,
            local: show(la, lp),
            remote: show(ra, rp),
            // UDP has no state machine worth showing.
            state: if proto.is_udp() {
                String::new()
            } else {
                state_name(st).to_string()
            },
            inode: f[9].parse().unwrap_or(0),
            pid: None,
            uid: f[7].parse().unwrap_or(0),
            process: String::new(),
        });
    }
    out
}

/// Read every table under a proc root.
pub fn read_all(proc: &Path) -> Vec<Conn> {
    let mut out = Vec::new();
    for (file, proto) in [
        ("net/tcp", Proto::Tcp),
        ("net/tcp6", Proto::Tcp6),
        ("net/udp", Proto::Udp),
        ("net/udp6", Proto::Udp6),
    ] {
        if let Ok(t) = std::fs::read_to_string(proc.join(file)) {
            out.extend(parse(&t, proto));
        }
    }
    out
}

/// The socket inode → (pid, process name) map, from what `/proc/*/fd` lets
/// us read. Unreadable processes are skipped silently — that is the normal
/// case for anything not ours.
pub fn inode_map(proc: &Path) -> HashMap<u64, (u32, String)> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(proc) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let dir = e.path();
        let Ok(fds) = std::fs::read_dir(dir.join("fd")) else {
            continue; // EACCES: not ours.
        };
        let comm = std::fs::read_to_string(dir.join("comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let Some(t) = target.to_str() else { continue };
            // `socket:[12345]`
            if let Some(inode) = t
                .strip_prefix("socket:[")
                .and_then(|r| r.strip_suffix(']'))
                .and_then(|r| r.parse::<u64>().ok())
            {
                out.insert(inode, (pid, comm.clone()));
            }
        }
    }
    out
}

/// Attach owners to rows; returns how many got one.
pub fn attribute(rows: &mut [Conn], map: &HashMap<u64, (u32, String)>) -> usize {
    let mut n = 0;
    for r in rows.iter_mut() {
        if let Some((pid, comm)) = map.get(&r.inode) {
            r.pid = Some(*pid);
            r.process = comm.clone();
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/net/proc/net")
            .join(name);
        std::fs::read_to_string(p).expect("the fixture reads")
    }

    #[test]
    fn addresses_decode_little_endian_in_both_families() {
        let (a, p) = parse_addr("0164A8C0:0016", false).unwrap();
        assert_eq!(a, IpAddr::V4(Ipv4Addr::new(192, 168, 100, 1)));
        assert_eq!(p, 22, "the port is big-endian hex");
        let (a, p) = parse_addr("00000000:0000", false).unwrap();
        assert_eq!(a, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(p, 0);
        // ::1 is fifteen zero bytes then 1 — four little-endian words.
        let (a, p) = parse_addr("00000000000000000000000001000000:1F90", true).unwrap();
        assert_eq!(a, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(p, 8080);
        assert!(parse_addr("nonsense", false).is_none());
        assert!(parse_addr("00:0", true).is_none(), "a short v6 address");
        assert_eq!(state_name(0x0A), "LISTEN");
        assert_eq!(state_name(0xFF), "UNKNOWN");
    }

    #[test]
    fn torchs_tables_parse_with_states_and_inodes() {
        let tcp = parse(&fixture("tcp"), Proto::Tcp);
        assert!(!tcp.is_empty());
        assert!(tcp.iter().all(|c| c.inode > 0 || c.state == "TIME-WAIT"));
        assert!(
            tcp.iter()
                .any(|c| c.state == "LISTEN" || c.state == "ESTAB"),
            "real states: {:?}",
            tcp.iter().map(|c| &c.state).collect::<Vec<_>>()
        );
        assert!(tcp.iter().all(|c| c.local.contains(':')));
        let udp = parse(&fixture("udp"), Proto::Udp);
        assert!(udp.iter().all(|c| c.state.is_empty()), "udp has no state");
        let tcp6 = parse(&fixture("tcp6"), Proto::Tcp6);
        assert!(tcp6.iter().all(|c| c.local.starts_with('[')));
        // A junk table yields nothing rather than panicking.
        assert!(parse("", Proto::Tcp).is_empty());
        assert!(parse("header\nshort\n", Proto::Tcp).is_empty());
    }

    /// The inode map over this process's own `/proc/self` — the one process
    /// we are guaranteed to be able to read.
    #[test]
    fn the_inode_map_attributes_our_own_sockets() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a socket");
        let port = listener.local_addr().unwrap().port();
        let map = inode_map(Path::new("/proc"));
        assert!(!map.is_empty(), "at least our own fds are readable");
        let mut rows = read_all(Path::new("/proc"));
        let n = attribute(&mut rows, &map);
        assert!(n > 0, "some socket is ours");
        // Our listener is in the table, attributed to this process.
        let me = std::process::id();
        let ours = rows
            .iter()
            .find(|r| r.local.ends_with(&format!(":{port}")))
            .expect("our listener is in /proc/net/tcp");
        assert_eq!(ours.pid, Some(me));
        assert_eq!(ours.state, "LISTEN");
        assert!(!ours.process.is_empty());
        drop(listener);
        // A tree with no processes at all.
        assert!(inode_map(Path::new("/nonexistent")).is_empty());
        assert!(read_all(Path::new("/nonexistent")).is_empty());
    }
}
