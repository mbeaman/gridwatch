<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Network-monitoring component for opsTui: data sources (interface, connection, per-process, latency), privileges, rendering plan by footprint, and prior art (Linux workstation "torch")

## TL;DR

Build the network component on **procfs 0.18 + sysfs + a few netlink/D-Bus reads**, not on sysinfo's `Networks` (it omits drop counters and re-runs `getifaddrs` on every refresh). Ship three privilege tiers, all detected at runtime with the `caps` crate: **Tier 0 (no privileges, default)** = interface rates, link/Wi-Fi state, addresses/routes/DNS, connection table with PIDs for the user's own processes, ICMP + TCP-connect probes; **Tier 1 (`cap_net_raw` on a small helper binary)** = per-process bandwidth via AF_PACKET sniffing (bandwhich/nethogs style); **Tier 2 (future, eBPF)** = per-process TCP/UDP accounting. Defer Tier 1/2 to a later arc; the Tier 0 component alone matches or exceeds bmon/gping and covers ~80 % of what the user sees in nethogs/iftop.

## Ground truth on this machine (all read-only, verified today)

- `/proc/net/dev` lists 9 interfaces: `lo, wlp7s0, eno1, eno2, virbr0, docker0, br-bc2d57ae738d, br-6bb7413a559e, vethcb88e24`. Line format is `"%6s:"` + 16 counters; names longer than 6 chars butt against the colon (`br-6bb7413a559e:      84`) so split on the first `:` not on whitespace.
- `eno1` currently negotiates **1000 Mb/s full** (sysfs `speed=1000`, `ethtool` agrees) even though the NIC is 2.5GbE - read speed from sysfs, never hardcode. `speed` is `-1` on `NO-CARRIER` bridges, `10000` on veth/bridges with carrier, and the read fails with EINVAL on the down Wi-Fi NIC - treat any error/negative as "unknown".
- `/sys/class/net/eno1/statistics/` exposes 24 counters (`rx_bytes, tx_bytes, rx_packets, tx_packets, rx_errors, tx_errors, rx_dropped, tx_dropped, rx_missed_errors, rx_nohandler, multicast, collisions, ...`); reading 8 of them costs 0.38 ms in Python vs 0.11 ms for one `/proc/net/dev` read. `carrier_changes/carrier_up_count/carrier_down_count` exist (2/1/1 on eno1) - nice for a "flaps" indicator.
- `net.ipv4.ping_group_range = 0 2147483647` (set by systemd's `/usr/lib/sysctl.d/50-default.conf`), so **unprivileged `SOCK_DGRAM/IPPROTO_ICMP` works**: a hand-rolled echo to 192.168.100.1 from Python returned in 1.4 ms; ICMPv6 DGRAM socket creation also succeeds; `SOCK_RAW` and `AF_PACKET` both fail with EPERM. `/usr/bin/ping` and `/usr/bin/mtr-packet` carry `cap_net_raw=ep`.
- Default route: `/proc/net/route` row `eno1 00000000 0164A8C0 ...` = gateway 192.168.100.1 (little-endian hex). `ip -j route get 1.1.1.1` returns `{"gateway":"192.168.100.1","dev":"eno1","prefsrc":"192.168.100.154"}`. No IPv6 default route and no global IPv6 address.
- DNS: `/etc/resolv.conf` is the systemd-resolved stub (127.0.0.53); real servers via `resolvectl --json=short dns` (works) or D-Bus `org.freedesktop.resolve1 /org/freedesktop/resolve1 Manager.DNS` = `a(iiay)` → ifindex 3, AF_INET, 192.168.100.1. `nsswitch hosts:` includes `mdns4_minimal`, so `getnameinfo()` may block on mDNS for RFC1918 addresses.
- Connections: `/proc/net/{tcp,tcp6,udp,udp6}` are world-readable (100 rows total right now). Scanning `/proc/*/fd` as the user: **628 processes, 143 readable / 482 EACCES, 6,360 fds, 990 socket inodes, 9.8 ms in Python** (Rust will be ~2-4 ms); joining with /proc/net tables took 1.25 ms and attributed 87/103 sockets. `ss -tunapH` shows `users:(("steam",pid=...,fd=...))` only for the user's own sockets; `ss -tunapHe` adds `ino:` and `uid:`; `ss -ti` returns full `tcp_info` (rtt, minrtt, cwnd, delivery_rate) **unprivileged** via INET_DIAG (kernel `CONFIG_INET_DIAG=m`). `ss` has no JSON mode.
- Wi-Fi: `wlp7s0` is down; `/proc/net/wireless` has header only; `iw dev wlp7s0 link` → "Not connected"; NetworkManager exposes it at `/org/freedesktop/NetworkManager/Devices/4` (`Device.Wireless.Bitrate=0`, `ActiveAccessPoint="/"`, `Device.State=20` = UNAVAILABLE). `iw 6.17`, `nmcli 1.54.3`, `wpa_supplicant` and NM are on the system bus. Kernel has `CONFIG_CFG80211_WEXT=y` so `/proc/net/wireless` will populate when connected.
- eBPF/conntrack: `kernel.unprivileged_bpf_disabled=2`, `/sys/kernel/btf/vmlinux` present, `bpftool` present, **no clang/bpf-linker**; `nf_conntrack` loaded but `/proc/net/nf_conntrack` absent (`CONFIG_NF_CONNTRACK_PROCFS` unset) and `nf_conntrack_acct=0`. `yama.ptrace_scope=1`.
- Packages: runtime `libpcap0.8t64`, `libnl-3-200`, `libnl-genl-3-200` installed; `libpcap-dev` NOT installed (candidate 1.10.6). `setcap/getcap` present. `/etc/services` present (365 lines).

## 1. Interface level

**Counters.** Read `/proc/net/dev` once per tick (`procfs::net::dev_status() -> ProcResult<HashMap<String, DeviceStatus>>`, fields `recv_bytes, recv_packets, recv_errs, recv_drop, recv_fifo, recv_frame, recv_compressed, recv_multicast, sent_bytes, sent_packets, sent_errs, sent_drop, sent_fifo, sent_colls, sent_carrier, sent_compressed`, all `u64`). Keep the last sample per interface and compute rates from monotonic timestamps:

```rust
pub struct IfaceSample { pub at: Instant, pub rx_bytes: u64, pub tx_bytes: u64, pub rx_pkts: u64, pub tx_pkts: u64, pub rx_err: u64, pub tx_err: u64, pub rx_drop: u64, pub tx_drop: u64 }
pub struct IfaceRate { pub rx_bps: f64, pub tx_bps: f64, pub rx_pps: f64, pub tx_pps: f64, pub err_delta: u64, pub drop_delta: u64 }
fn rate(prev: &IfaceSample, cur: &IfaceSample) -> IfaceRate {
    let dt = cur.at.duration_since(prev.at).as_secs_f64().max(1e-3);
    let d = |a: u64, b: u64| a.saturating_sub(b) as f64 / dt;   // saturating: counters reset on re-create
    IfaceRate { rx_bps: d(cur.rx_bytes, prev.rx_bytes), tx_bps: d(cur.tx_bytes, prev.tx_bytes), /* ... */ }
}
```
Sample at 1 s for the table and 250 ms for sparklines only if the theme wants it - at 250 ms the per-tick byte deltas on a 1 GbE link are ~30 MB max, so use an EWMA (alpha 0.3-0.5) or a 1 s sliding window to avoid a jittery sparkline. Keep a ring buffer of 120-600 samples per interface (`VecDeque<u64>`), plus a running `max` for the sparkline's `.max()`.

**sysinfo 0.39.6 `Networks`** (verified on docs.rs and in `src/unix/linux/network.rs`): `Networks::new_with_refreshed_list()`, `refresh(&mut self, remove_not_listed_interfaces: bool)`, `list() -> &HashMap<String, NetworkData>`. `NetworkData::received()/transmitted()/packets_*()/errors_on_*()` are **deltas since the previous refresh** (you still divide by your own elapsed time), `total_*()` are cumulative, plus `mac_address() -> MacAddr`, `ip_networks() -> &[IpNetwork{addr: IpAddr, prefix: u8}]`, `mtu() -> u64`, `operational_state() -> InterfaceOperationalState{Up,Down,Testing,Unknown,Dormant,NotPresent,LowerLayerDown}`. Linux impl reads only `rx_bytes,tx_bytes,rx_packets,tx_packets,rx_errors,tx_errors` + `mtu` + `operstate` from sysfs, calls `getifaddrs` on every refresh, includes `lo` and never reads `rx_dropped/tx_dropped`. Verdict: fine for the htop component's system stats, but hand-roll the network source (drops matter for a 2.5GbE NIC and for bridges - `virbr0/docker0` already show `tx_dropped=61`).

**Link state.** Per interface read `/sys/class/net/<if>/{operstate,carrier,speed,duplex,mtu,address,type,flags,carrier_changes}` every ~2 s (cheap, but not every 250 ms). `type` 1 = ARPHRD_ETHER, 772 = loopback; `flags` bit 0x1 UP, 0x40 RUNNING (0x1003 on eno1). `/sys/class/net/<if>/wireless` and `phy80211` exist only for Wi-Fi NICs - use their presence as the Wi-Fi detector. Alternative single-shot source: `rtnetlink 0.23` `RTM_GETLINK` dump gives `LinkAttribute::{IfName, Mtu, Address, OperState(State), Carrier(u8), CarrierUpCount, CarrierDownCount, Stats64(Stats64{rx_bytes, tx_bytes, rx_dropped, tx_dropped, rx_missed_errors, rx_nohandler, ...})}` in one syscall (this is what bmon does via libnl); `ip -j -s link` returns the same `stats64` JSON if you prefer a subprocess. Not worth the async netlink dependency in arc 1.

**Wi-Fi (nl80211).** Options, best first:
1. `neli-wifi 0.6.1` (sync `Socket` or tokio `AsyncSocket`): `Socket::connect()`, `get_interfaces_info(&mut self) -> Result<Vec<Interface>>` with `Interface{index: Option<i32>, ssid: Option<Vec<u8>>, mac, name: Option<Vec<u8>>, frequency: Option<u32>, channel, power, phy, device}`, `get_station_info(&mut self, if_index: i32) -> Result<Vec<Station>>` with `Station{signal: Option<i8> /*dBm*/, average_signal: Option<i8>, rx_bitrate: Option<u32>, tx_bitrate: Option<u32>, connected_time, beacon_loss, bssid: Option<Vec<u8>>, tx_failed, tx_retries, ht_mcs, vht_mcs, he_mcs, eht_mcs}`. Bitrates are nl80211 `RATE_INFO_BITRATE32` units = **100 kbit/s** (confirmed on wl-nl80211's `Nl80211RateInfo::Bitrate32(u32)` docs: "100kb/s"), so display `bitrate as f64 / 10.0` Mbit/s. No system libs, no privileges (GET_STATION is unprivileged). Pulls neli 0.6 (the standalone neli is 0.7.4 - accept the duplicate).
2. `wl-nl80211 0.7.0` (rust-netlink org, tokio): `let (conn, handle, _) = wl_nl80211::new_connection()?; tokio::spawn(conn); let mut s = handle.station().dump(if_index).execute().await; while let Some(msg) = s.try_next().await? {...}`. Lower-level, 48 % documented; use only if you already adopt rust-netlink for rtnetlink.
3. NetworkManager over zbus (already a project dependency): `org.freedesktop.NetworkManager.Device.Wireless` `ActiveAccessPoint (o)`, `Bitrate (u, kbit/s)`; `AccessPoint` `Ssid (ay)`, `Strength (y, percent)`, `Frequency (u, MHz)`, `MaxBitrate (u, kbit/s)`, `HwAddress (s)`. No dBm, but PropertiesChanged signals give you push updates for free.
4. `/proc/net/wireless` (WEXT compat, present here): format `"%6s: %04x  %3d%c  %3d%c  %3d%c  %6d %6d %6d %6d %6d   %6d"` = status, quality(/70 on mac80211), level dBm, noise, then discard counters; the trailing `.` means "updated". Cheapest, but WEXT is legacy - fine as a fallback.
5. `iw dev <if> link` / `station dump` subprocess - avoid in the tick loop (fork per second); acceptable for a one-shot "details" popup.

**Interface filtering.** Default hide-list by glob: `lo`, `veth*`, `br-*`, `docker*`, `virbr*`, `vnet*`, `tap*`, `dummy*`, `bond*` slaves; default show physical + VPN: `en*`, `eth*`, `wl*`, `ww*`, `tun*`, `wg*`, `tailscale*`, `ppp*`. Make it a config list of globs (`[net] show = ["en*","wl*","wg*"] hide = ["veth*",...]`) with a keybinding to toggle "all". Also auto-collapse interfaces with `operstate=down` and zero traffic into a one-line "4 hidden" footer. Bridges/veths churn (docker up/down) - key the sample map by name and reset the previous sample when `ifindex` changes.

**Addresses.** `nix 0.31.3` (feature `net`): `nix::ifaddrs::getifaddrs() -> Result<InterfaceAddressIterator>` yielding `InterfaceAddress{interface_name, flags: InterfaceFlags, address: Option<SockaddrStorage>, netmask, broadcast, destination}`. Simpler: `if-addrs 0.15.0` `get_if_addrs() -> io::Result<Vec<Interface{name, addr: IfAddr::V4(Ifv4Addr{ip, netmask, prefixlen, broadcast})|V6(..), index: Option<u32>, oper_status}>>` with `is_loopback()/is_link_local()/ip()` (libc getifaddrs underneath). Or parse `/proc/net/if_inet6` (`addr(32 hex) ifindex prefixlen scope flags name`; scope 0x20 = link-local, 0x00 = global). Refresh every 5-10 s or on rtnetlink RTM_NEWADDR.

**Default route / gateway.** `procfs::net::route() -> Vec<RouteEntry{iface: String, destination: Ipv4Addr, gateway: Ipv4Addr, flags: u16, metrics: u32, mask: Ipv4Addr, ...}>` - the default route is `destination == 0.0.0.0 && flags & 0x2 (RTF_GATEWAY)`; pick lowest metric. For IPv6 parse `/proc/net/ipv6_route` (dest/prefix `00` = default). `ip -j route get 1.1.1.1` is the simplest "what would I use" probe if a subprocess is acceptable at startup.

**DNS servers.** Prefer zbus `org.freedesktop.resolve1` `Manager.DNS` (`a(iiay)`: ifindex, family, address bytes) and `CurrentDNSServer`; fall back to parsing `/etc/resolv.conf` with the `resolv-conf` crate (the value here is the stub 127.0.0.53, so label it "via systemd-resolved"). `resolvectl --json=short dns` works as a subprocess fallback.

**Public IP (opt-in, off by default).** HTTPS: `https://api.ipify.org?format=json` / `api64.ipify.org` (documented "no limit"), or `https://1.1.1.1/cdn-cgi/trace`. DNS-only (no HTTP client): TXT `whoami.cloudflare` class CH at 1.1.1.1, A `myip.opendns.com` at resolver1.opendns.com. Re-query at most every 15 min and on default-route change; cache in state; show a privacy note in config. Use `ureq 3.4` (blocking, spawn_blocking) or a hickory TXT lookup; the `public-ip 0.2.2` crate still depends on trust-dns 0.20 / hyper 0.14 - skip it.

## 2. Connection level

`/proc/net/tcp` columns (kernel `proc_net_tcp.txt`): `sl local_address rem_address st tx_queue:rx_queue tr:tm->when retrnsmt uid timeout inode ...`, addresses are little-endian hex IPv4 (`0164A8C0` = 192.168.100.1) / 4×32-bit little-endian words for IPv6, ports big-endian hex, `st` hex = `TcpState{Established=1, SynSent, SynRecv, FinWait1, FinWait2, TimeWait, Close, CloseWait, LastAck, Listen=0xA, Closing, NewSynRecv=0xC}`. `procfs::net::tcp()/tcp6()/udp()/udp6()` return `Vec<TcpNetEntry{local_address: SocketAddr, remote_address: SocketAddr, state: TcpState, rx_queue: u32, tx_queue: u32, uid: u32, inode: u64}>` (UDP analogue `UdpNetEntry`). `/proc/net` is per net-namespace: containers' sockets are invisible unless you read `/proc/<container-pid>/net/tcp` (`Process::tcp()`), which needs root for other users' processes.

**inode → PID** (bandwhich's `src/os/linux.rs` shape, verified):
```rust
let mut inode_to_proc: HashMap<u64, ProcInfo> = HashMap::new();
for p in procfs::process::all_processes()?.filter_map(Result::ok) {
    let Ok(fds) = p.fd() else { continue };            // EACCES for other users' processes
    let name = p.stat().map(|s| s.comm).unwrap_or_default();
    for fd in fds.filter_map(Result::ok) {
        if let FDTarget::Socket(inode) = fd.target { inode_to_proc.insert(inode, ProcInfo{pid: p.pid, name: name.clone()}); }
    }
}
```
Cost here: ~10 ms/scan for 6.4 k fds; run it at 1-2 s cadence on a blocking thread, and only when the connection table is visible (6x3/full). Without `CAP_SYS_PTRACE`+`CAP_DAC_READ_SEARCH` you only resolve the user's own processes (143 of 628 here) - which is what the user cares about on a workstation (steam, firefox, claude). Root-owned sockets (dnsmasq, resolved, NM) render as `uid:0` with no PID; show the `uid` column from `/proc/net/tcp` instead of blank.

**sock_diag (netlink INET_DIAG)** is the better-than-/proc path: `netlink-sys 0.9` + `netlink-packet-sock-diag 0.5` (`SockDiagMessage::InetRequest{family, protocol, extensions: ExtensionFlags::INFO|MEMINFO, states: StateFlags::all(), ..}` with `NLM_F_REQUEST|NLM_F_DUMP`; responses `InetResponse{header: InetResponseHeader{socket_id{source_address, source_port, destination_*, interface_id, cookie}, state, uid, inode, recv_queue, send_queue, ...}, nlas}`). It is unprivileged (confirmed by `ss -ti`) and `Nla::TcpInfo(Vec<u8>)` carries raw `struct tcp_info` (parse `tcpi_rtt`/`tcpi_min_rtt` in µs, `tcpi_bytes_acked/received`, `tcpi_delivery_rate` yourself - the crate leaves it as bytes). This gives **per-connection RTT and throughput** (a distinctive feature no TUI competitor shows), but it is a second arc item; `/proc/net/tcp` is enough for arc 1.

**`ss -tunapH` subprocess**: same information, `users:(("name",pid=N,fd=M))` only for own processes, `-e` adds `ino:`/`uid:`; parseable but fork-per-tick and column widths vary. Use only as a debug cross-check.

**Labelling.** Service names: parse `/etc/services` once (`ssh 22/tcp`, `https 443/udp` for HTTP/3) into `HashMap<(u16, Proto), &str>`. Reverse DNS: copy bandwhich's design - `hickory-resolver 0.26.1` (`Resolver::builder_tokio()?.build()`, `reverse_lookup(ip).await`, first answer → `String`; `is_no_records_found()` → cache the IP string so it is never retried), a `HashSet<IpAddr>` of pending lookups, an `Arc<Mutex<HashMap<IpAddr, Option<String>>>>` cache with a 10-minute negative TTL, and a bounded mpsc queue. Avoid `dns-lookup::lookup_addr` (`getnameinfo`, blocking) because `nsswitch` has `mdns4_minimal` here - it can stall 1-2 s per RFC1918 address. Skip lookups for loopback/link-local/multicast and make reverse DNS opt-out (it leaks the remote list to the LAN resolver).

## 3. Per-process bandwidth (nethogs-style) - defer

What it takes: attribute *bytes on the wire* to a process. Three routes, none unprivileged:
- **AF_PACKET sniffing** (bandwhich: `pnet_datalink 0.35` `channel(&iface, Config{read_timeout: Some(1s), read_buffer_size: 65536, ..})` → `Channel::Ethernet(tx, rx)`, no libpcap needed on Linux; nethogs/iftop/sniffnet: libpcap via `pcap 2.5` which needs `libpcap-dev` to link - not installed). Needs `cap_net_raw` (+`cap_net_admin` for promiscuous mode); attributing to other users' PIDs additionally needs `cap_sys_ptrace,cap_dac_read_search` (bandwhich's exact setcap line). Costs one thread per interface parsing every frame (`etherparse 0.21`), and 5-tuple → inode → pid joins on a 1-2 s cadence; UDP/QUIC (Steam, HTTP/3) attributes fine, but short-lived sockets land in "unknown" (nethogs documents this).
- **eBPF** (`aya 0.14`, BCC `tcptop` style kprobes on `tcp_sendmsg`/`tcp_recvmsg` keyed by pid/comm/5-tuple): exact per-PID byte counts with no packet copies, but requires `CAP_BPF+CAP_PERFMON` (or root; `unprivileged_bpf_disabled=2` here), a nightly toolchain + `bpf-linker` (neither installed), and an eBPF object shipped with the binary. Kernel 7.0 with BTF is ideal for CO-RE. Best long-term answer, worst arc-1 answer.
- **nf_conntrack accounting**: per-flow only (never per-process), `nf_conntrack_acct=0`, no procfs table, needs `CAP_NET_ADMIN` for the netlink dump. Not useful.

Recommendation: **defer to a dedicated arc** and design for it now. Capability model (mirrors trippy's `trippy-privilege`, verified: `caps::has_cap(None, CapSet::Permitted, Capability::CAP_NET_RAW)` → `caps::raise(None, CapSet::Effective, ..)` → open sockets → `caps::clear(None, CapSet::Effective)`): at startup compute a `NetCaps { raw_socket: bool, ptrace_proc: bool, bpf: bool }` and surface it in the component header ("per-process: needs cap_net_raw - run `opstui net-helper install`"). Prefer a **separate helper binary** (`opstui-netd`, ~few hundred lines: sniff, aggregate per (pid, iface) byte counters, stream over a `$XDG_RUNTIME_DIR` unix socket) with `setcap cap_net_raw,cap_sys_ptrace,cap_dac_read_search+p` over setcap on the TUI itself: the TUI parses themes/config/plugins and talks to PipeWire, D-Bus and NVML, so keep the elevated surface tiny; file capabilities also vanish on every `cargo build`/`cargo install`, which is painful during development but harmless for a helper installed once. Third option with precedent in astral-watch: a systemd unit with `AmbientCapabilities=CAP_NET_RAW` exporting over a socket/Prometheus.

## 4. Latency / quality

- **ICMP**: `surge-ping 0.9.0` - `Config::builder().kind(ICMP::V4).build()`; default `sock_type_hint = Type::DGRAM` and `Client::new(&Config) -> io::Result<Client>` falls back to `Type::RAW` if DGRAM fails (verified in `client.rs`); `client.pinger(ip, PingIdentifier(n)).await -> Pinger`; `pinger.timeout(Duration)`; `pinger.ping(PingSequence(seq), &[0u8; 56]).await -> Result<(IcmpPacket, Duration), SurgeError>`. One `Client` (one socket) cloned across targets; replies demuxed by `(ip, Option<ident>, seq)` - the `Option` matters because the kernel rewrites the ICMP id on DGRAM sockets (verified: sent id 1, reply id 57270). With this machine's `ping_group_range` it works unprivileged; on hosts with the kernel default `1 0` it needs `cap_net_raw` - detect `EPERM` at startup and fall back.
- **TCP-connect probes** (no privilege anywhere): `tokio::time::timeout(1s, TcpStream::connect((ip, 443|53)))`, measure `Instant` delta, immediately drop the socket. Gateway:53 answered in 3.45 ms here. Use for 1.1.1.1:443/8.8.8.8:443 when ICMP is unavailable or filtered.
- **Probe set**: gateway (from the default route, re-resolved on route change), 1.1.1.1, 8.8.8.8, optional 9.9.9.9 and a user list; 1 Hz, 1 s timeout, 56-byte payload, IPv6 twins only when a global v6 address exists (none now).
- **Stats** per target over a 60-sample ring: last, min, avg, max, `mdev` (iputils-style std-dev), **jitter** as RFC 3550 smoothed `J += (|D| - J)/16` of consecutive RTT differences, loss % = timeouts/sent, and a "streak" of consecutive losses to colour the tile. Keep the gateway probe as the "LAN" number and the internet targets as "WAN".
- **Hop tracing**: `trippy-core 0.13.0` is a real library (socket2 raw sockets, requires `CAP_NET_RAW`; unprivileged mode is macOS-only per trippy.rs) - a later full-view "trace" toggle, not arc 1. `gping` uses `pinger 2.1.4`, which just spawns `/bin/ping` and regex-parses it - do not copy that.

## 5. Rendering plan by size class (grid cells; fonts are DejaVu/Noto, no Nerd Fonts - arrows `↓ ↑`, braille `⣿`, blocks `▁▂▃▄▅▆▇█` are all present)

- **1x1**: two lines, `↓ 12.4 MB/s` / `↑ 812 kB/s` for the primary interface (default-route dev), colour by utilisation vs `speed`; a 1-char link dot (`●` up / `○` down).
- **2x1**: rates + two `Sparkline`s (`ratatui 0.30.2`: `.data(&[Option<u64>])`, `.max(peak)`, `.direction(RenderDirection::RightToLeft)`, `.absent_value_symbol(" ")`), 30-60 s window, rx above tx; status line with `1G FD`/SSID + dBm.
- **4x2**: `Table` of shown interfaces (`if, state, speed, ↓rate, ↑rate, drops/errs Δ, addr`) plus a dual braille `Chart` (`Dataset::data(&[(f64,f64)]).marker(Marker::Braille).graph_type(GraphType::Line)` or `GraphType::Area` with `fill_to_y(0.0)` for the retrowave theme), rx positive / tx mirrored below zero, auto-scaled y with human units; probe strip `gw 1.4ms  1.1.1.1 9ms  8.8.8.8 11ms  loss 0%`.
- **6x3**: adds "top connections" (top 8 by rx_queue/state or, in Tier 1, by bytes/s): `proc  local:port → remote(name):svc  state  rtt`.
- **full**: full connection table with sort keys (`p` proc, `r` remote, `s` state, `b` bytes when available), filter by protocol/state/text, `d` toggles reverse DNS, `a` toggles hidden interfaces, a detail pane for the selected interface (addresses, gateway, DNS, carrier flaps, Wi-Fi MCS/bitrate) and, when the helper is present, per-process bars nethogs-style; probe pane with per-target sparklines and jitter/loss.

Suggested module shape: `net/source/{dev.rs (proc/net/dev), link.rs (sysfs), wifi.rs (neli-wifi | NM), addrs.rs, route.rs, dns.rs, conns.rs (procfs + inode map), probe.rs (surge-ping + tcp), pubip.rs}` feeding one `NetSnapshot` struct via a tokio task at 1 Hz (plus a 250 ms sub-tick for counters if the theme wants it), rendered by `NetWidget::render(size_class, &NetSnapshot, &Theme)`.

## 6. Prior art

| Tool | Data path | Privilege | Reusable |
|---|---|---|---|
| bandwhich 0.23.1 (Rust) | `pnet_datalink` AF_PACKET + `procfs` inode join, `hickory-resolver` rDNS, ratatui 0.30 | `setcap cap_sys_ptrace,cap_dac_read_search,cap_net_raw,cap_net_admin+ep` | Binary-only crate (no `[lib]`); copy the `os/linux.rs` join and `dns/` cache design, not the code |
| nethogs 0.8.8 (C++) | libpcap + /proc/net/tcp + /proc/*/fd | same four caps (`+pe`) | `libnethogs` exists but "experimental, may change" |
| iftop (C) | libpcap per-host flows | cap_net_raw | look only |
| bmon 4.0 (C) | libnl-3/libnl-route-3 rtnetlink link stats | none | model for rtnetlink `Stats64` and its rate/graph layout |
| gping 1.20 (Rust) | `pinger` crate → spawns `ping` | inherits ping's caps | avoid the approach |
| trippy 0.13 (Rust) | socket2 raw ICMP/UDP/TCP, `trippy-privilege` (caps crate) | `setcap CAP_NET_RAW+p` | `trippy-core` library + the privilege pattern |
| sniffnet 1.5 (Rust, iced) | `pcap` crate (needs libpcap-dev) + etherparse | `setcap cap_net_raw,cap_net_admin=eip` | look only |

## Recommendations

- **Hand-roll the interface source on /proc/net/dev (procfs::net::dev_status) + sysfs link attributes; do not use sysinfo::Networks for the network component.** — sysinfo 0.39 reads only rx/tx bytes/packets/errors (no dropped counters), runs getifaddrs on every refresh, includes lo, and still leaves rate computation to the caller. /proc/net/dev is one 0.1 ms read with all 16 counters; drops are already non-zero on the bridges here.
  - alternatives: sysinfo::Networks if the htop component already refreshes it and you accept no drop counters; rtnetlink 0.23 RTM_GETLINK Stats64 for a single-syscall, bmon-style path.
- **Sample counters at 1 Hz (optionally 250 ms for sparklines with EWMA/1 s window), compute rates from Instant deltas with saturating_sub, and key per-interface state by (name, ifindex) so docker veth churn resets cleanly.** — Counters are u64 and monotonic; interface re-creation is the only reset case. 250 ms deltas on a 1 GbE link are noisy without smoothing.
  - alternatives: Fixed 1 Hz only (simpler, matches bmon/htop).
- **Ship a Tier 0 (unprivileged) component in arc 1: rates, link/Wi-Fi state, addresses, default route/gateway, DNS, connection table with own-process PIDs, ICMP + TCP-connect probes. Defer per-process bandwidth to a separate arc.** — Everything in Tier 0 is verified to work as the user on this machine (ping_group_range open, /proc/net/* world-readable, own /proc/*/fd readable). Per-process needs cap_net_raw (AF_PACKET) or CAP_BPF and a nightly/bpf-linker toolchain that is not installed.
  - alternatives: Setcap the main binary and do bandwhich-style sniffing in arc 1 (fast to build, but caps are lost on every cargo build and the whole TUI runs elevated).
- **Design the capability model now: detect CAP_NET_RAW/CAP_SYS_PTRACE/CAP_BPF with the caps crate (has_cap on the Permitted set, raise to Effective only around socket creation, then clear), surface the tier in the widget header, and plan a tiny setcap'd helper binary (opstui-netd) over a unix socket rather than elevating the TUI.** — Mirrors trippy-privilege's verified pattern; keeps the elevated surface small; the helper survives rebuilds of the TUI. astral-watch's exporter/service is precedent for an out-of-process data source.
  - alternatives: systemd unit with AmbientCapabilities=CAP_NET_RAW exporting Prometheus/JSON; or eBPF (aya) directly once a nightly+bpf-linker toolchain is acceptable.
- **Use surge-ping 0.9 for ICMP (DGRAM default, RAW fallback) and tokio TcpStream::connect with timeout as the no-privilege fallback; probe gateway + 1.1.1.1 + 8.8.8.8 at 1 Hz with 60-sample ring stats (min/avg/max/mdev, RFC3550 jitter, loss %).** — Unprivileged DGRAM ICMP verified on this host (1.4 ms to gateway); surge-ping multiplexes many targets on one socket and tolerates the kernel-rewritten ICMP id. TCP-connect verified at 3.45 ms to gateway:53.
  - alternatives: pinger crate (spawns /bin/ping, regex parsing - not recommended); trippy-core for hop tracing later (needs CAP_NET_RAW).
- **Connection table from procfs::net::{tcp,tcp6,udp,udp6} joined with an inode->pid map built from procfs::process::all_processes()/fd() on a blocking thread at 1-2 s, only while the table is visible; show uid for sockets whose owner is unreadable.** — Scan is ~10 ms in Python (less in Rust) for 6.4k fds; 143/628 processes readable as the user, which covers the user's apps. Same approach as bandwhich.
  - alternatives: ss -tunapHe subprocess (fork per tick, fragile columns); netlink sock_diag via netlink-packet-sock-diag 0.5 for a later arc that adds per-connection RTT from raw tcp_info bytes.
- **Reverse DNS via hickory-resolver 0.26 (builder_tokio, reverse_lookup) with a pending set + HashMap cache + negative TTL, opt-out; service names from /etc/services; never call getnameinfo in the render path.** — nsswitch here has mdns4_minimal, so blocking getnameinfo can stall; bandwhich's cache design is verified and simple.
  - alternatives: dns-lookup::lookup_addr inside spawn_blocking with a per-call timeout.
- **Wi-Fi via neli-wifi 0.6.1 (nl80211 GET_INTERFACE/GET_STATION, no privileges, no C libs), with NetworkManager D-Bus (zbus, already planned) as the fallback for SSID/strength%; keep /proc/net/wireless as last resort.** — Gives dBm, average signal, rx/tx bitrate (100 kbit/s units), MCS and beacon loss directly; NM gives push updates but no dBm.
  - alternatives: wl-nl80211 0.7 (rust-netlink, tokio) if you standardise on rust-netlink; iw subprocess for a one-shot details popup.
- **Default interface filter: hide lo, veth*, br-*, docker*, virbr*, vnet*, tap*; show en*/eth*/wl*/ww*/tun*/wg*/tailscale*; configurable globs plus an 'a' toggle; auto-collapse down+idle interfaces into a footer count.** — This host has 5 noise interfaces vs 3 real NICs (one up). The sane default is 'what the user plugs in or VPNs over'.
  - alternatives: Show everything sorted by traffic (bmon style).
- **Public IP lookup opt-in, off by default, at most every 15 min or on default-route change, via api.ipify.org (or a DNS TXT query to whoami.cloudflare) with a privacy note; do not use the public-ip crate.** — It is an outbound request from a personal dashboard; ipify documents no rate limit; public-ip 0.2.2 pins trust-dns 0.20/hyper 0.14 (stale).
  - alternatives: 1.1.1.1/cdn-cgi/trace over HTTPS; myip.opendns.com A query.

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `procfs` | 0.18.0 | /proc/net/dev (net::dev_status), /proc/net/{tcp,tcp6,udp,udp6} (TcpNetEntry/UdpNetEntry), route(), arp(), process::all_processes()/Process::fd() with FDTarget::Socket(inode) for inode->pid mapping | none | verified |
| `sysinfo` | 0.39.6 | Optional: Networks/NetworkData (deltas since last refresh, totals, mac, ip_networks, mtu, operational_state). Not recommended for the net component (no drop counters). | none | verified |
| `surge-ping` | 0.9.0 | Async ICMP echo; Config defaults to SOCK_DGRAM with RAW fallback; Client::new(&Config) -> io::Result<Client>; client.pinger(ip, PingIdentifier).await; pinger.ping(PingSequence, &[u8]).await -> Result<(IcmpPacket, Duration), SurgeError> | none; unprivileged when net.ipv4.ping_group_range includes the user's gid (true here), else cap_net_raw | verified |
| `socket2` | 0.6.5 | Transitive via surge-ping; direct use only if hand-rolling DGRAM ICMP or TCP-connect probes with fine-grained options | none | likely |
| `tokio` | 1.53.1 | TcpStream::connect + time::timeout for no-privilege latency probes; runtime for hickory/surge-ping/neli-wifi async | none | verified |
| `hickory-resolver` | 0.26.1 | Reverse DNS (Resolver::builder_tokio()?.build(), reverse_lookup(ip).await) and TXT lookups (public IP via DNS); default features system-config + tokio | none | verified |
| `dns-lookup` | 4.0.1 | Alternative blocking getnameinfo: lookup_addr(&IpAddr) -> Result<String, LookupError> (beware mdns4_minimal stalls) | none | verified |
| `neli-wifi` | 0.6.1 | nl80211 without C libs: Socket::connect(), get_interfaces_info() -> Vec<Interface{index, ssid: Option<Vec<u8>>, frequency, ...}>, get_station_info(if_index) -> Vec<Station{signal: Option<i8> dBm, average_signal, rx_bitrate/tx_bitrate: Option<u32> in 100 kbit/s, beacon_loss, ...}>; AsyncSocket with tokio feature | none (pulls neli 0.6 alongside neli 0.7.4 if you also use neli directly) | verified |
| `wl-nl80211` | 0.7.0 | rust-netlink nl80211 (tokio/async-std): new_connection(), handle.station().dump(if_index).execute() stream; Nl80211RateInfo::Bitrate32(u32) in 100 kb/s | none | verified |
| `zbus` | 5.19.0 | NetworkManager (Device.Wireless.ActiveAccessPoint/Bitrate, AccessPoint.Ssid/Strength/Frequency) and systemd-resolved (Manager.DNS a(iiay), CurrentDNSServer) over D-Bus; already planned for MPRIS | none (pure Rust D-Bus) | verified |
| `nix` | 0.31.3 | nix::ifaddrs::getifaddrs() (feature "net") for IPv4/IPv6 addresses per interface | none | verified |
| `if-addrs` | 0.15.0 | Simpler getifaddrs wrapper: get_if_addrs() -> io::Result<Vec<Interface{name, addr: IfAddr::V4/V6, index: Option<u32>, oper_status}>> with is_loopback()/is_link_local()/ip() (docs.rs build failed for 0.15.0; API verified from source) | none | verified |
| `resolv-conf` | 0.7.6 | Parse /etc/resolv.conf fallback (as bandwhich does) | none | likely |
| `caps` | 0.5.6 | Capability detection/raise/drop: caps::has_cap(None, CapSet::Permitted, Capability::CAP_NET_RAW), caps::raise(None, CapSet::Effective, ..), caps::clear(None, CapSet::Effective) - the trippy-privilege pattern | none; file caps set with setcap (libcap2-bin, present) | verified |
| `netlink-packet-sock-diag` | 0.5.0 | Later arc: INET_DIAG dump (InetRequest/InetResponse with uid, inode, state, queues) unprivileged; Nla::TcpInfo(Vec<u8>) is raw struct tcp_info (parse rtt/min_rtt/delivery_rate yourself) | none (kernel CONFIG_INET_DIAG=m, loaded) | verified |
| `netlink-sys` | 0.9.0 | Raw NETLINK_SOCK_DIAG / NETLINK_ROUTE sockets (blocking or tokio) for the sock_diag/rtnetlink paths | none | verified |
| `rtnetlink` | 0.23.0 | Optional single-syscall link dump: LinkAttribute::{IfName, Mtu, Address, OperState, Carrier, CarrierUpCount/DownCount, Stats64(Stats64{rx_bytes, tx_bytes, rx_dropped, tx_dropped, rx_missed_errors, rx_nohandler, ...})} (netlink-packet-route 0.33.0) | none | verified |
| `pnet_datalink` | 0.35.0 | Tier 1 helper: AF_PACKET capture without libpcap (channel(&iface, Config{read_timeout, read_buffer_size, promiscuous, linux_fanout, ..}) -> Channel::Ethernet) | cap_net_raw (+cap_net_admin for promiscuous); no libpcap | verified |
| `etherparse` | 0.21.0 | Tier 1 helper: parse Ethernet/IPv4/IPv6/TCP/UDP headers from captured frames (used by sniffnet) | none | likely |
| `pcap` | 2.5.0 | libpcap alternative to pnet_datalink (what sniffnet/nethogs/iftop use); not recommended here | libpcap-dev to build (NOT installed; runtime libpcap0.8t64 is), cap_net_raw,cap_net_admin | verified |
| `aya` | 0.14.0 | Tier 2 (future): eBPF kprobes on tcp_sendmsg/tcp_recvmsg keyed by pid for exact per-process bytes (BCC tcptop model) | CAP_BPF+CAP_PERFMON or root (unprivileged_bpf_disabled=2 here); nightly toolchain + bpf-linker (not installed); BTF present | likely |
| `trippy-core` | 0.13.0 | Future 'trace' view: traceroute library (socket2 raw sockets) | cap_net_raw (unprivileged mode is macOS-only) | verified |
| `ureq` | 3.4.0 | Opt-in public IP HTTPS lookup (api.ipify.org?format=json) from spawn_blocking | none (rustls) | likely |
| `ratatui` | 0.30.2 | Sparkline (data accepts &[u64] / Option<u64> / SparklineBar; max(); direction(RenderDirection); absent_value_symbol), Chart/Dataset (data(&[(f64,f64)]), marker(Marker::Braille), graph_type(GraphType::Line/Area), fill_to_y), Table | none | verified |
| `bandwhich` | 0.23.1 | Prior art only: binary crate with no [lib]; pnet 0.35 + procfs 0.18 + hickory-resolver 0.26 + ratatui 0.30 | n/a | verified |
| `pinger` | 2.1.4 | What gping uses: spawns the system ping binary and regex-parses output - not recommended | iputils ping | verified |

## Risks

- **Privilege tiers make behaviour host-dependent: ping_group_range differs across distros (kernel default '1 0' disables DGRAM ICMP), and a setcap'd binary loses its file capabilities on every cargo build/install.** → Detect at startup (try DGRAM ICMP, caps::has_cap) and degrade to TCP-connect probes / 'uid only' columns with an explicit header badge; keep elevated code in a separate, rarely rebuilt helper binary; document the setcap line and offer `opstui net-helper install`.
- **/proc/*/fd scanning scales with process/fd count (628 procs, 6.4k fds today; a busy game or docker load could double it) and only resolves the user's own processes (143/628).** → Run on a blocking thread at 1-2 s only while the connection view is visible; cache pid->name; show uid from /proc/net/tcp for unresolvable sockets; consider sock_diag later.
- **/proc/net/tcp is per net-namespace: docker/libvirt guest connections are invisible, and bridge/veth interfaces show only host-side counters.** → Label the table 'host namespace'; optionally read /proc/<pid>/net/tcp for container init processes when running privileged in a later arc.
- **Reverse DNS leaks the remote-endpoint list to the LAN resolver and getnameinfo can stall on mDNS (nsswitch has mdns4_minimal).** → Opt-out toggle, async hickory resolver with pending set and negative cache, skip private/link-local ranges by default, never resolve on the render path.
- **The Wi-Fi code path cannot be tested now (wlp7s0 is down, no SSID); nl80211 attribute parsing and unit conversions (100 kbit/s bitrates, dBm signal) may be wrong until exercised.** → Unit-test against recorded nl80211 fixtures; add NetworkManager D-Bus fallback; validate once by connecting the mt7925 to an AP during the Wi-Fi arc.
- **Sparkline/chart noise at 250 ms sampling and misleading utilisation colours if link speed is read wrong (eno1 negotiates 1000 not 2500 today; sysfs speed is -1/EINVAL when down).** → Smooth with EWMA or 1 s window; treat speed <= 0 or read errors as unknown and fall back to autoscale; re-read sysfs link attrs on carrier_changes.
- **Opt-in public IP lookup and ICMP probes generate periodic outbound traffic that may surprise the user or a captive network.** → Off by default for public IP; probes on by default but with a config switch and a 1 Hz cap; pause probes when no default route exists.
- **Unicode rendering: braille and block glyphs render correctly in DejaVu Sans Mono/Ptyxis, but Nerd Font icons are unavailable and some themes may assume them.** → Theme symbol sets restricted to Unicode blocks/braille/arrows verified in DejaVu; keep an ASCII fallback set.
- **Interface churn (docker veth create/destroy) can produce negative deltas or stale rows.** → Key state by (name, ifindex), saturating_sub, drop rows absent for N ticks, and collapse hidden interfaces into a count.

## Verified facts

- eno1 link is 1000 Mb/s full duplex right now (cat /sys/class/net/eno1/speed = 1000, /duplex = full; ethtool eno1 Speed: 1000Mb/s) despite being a 2.5GbE NIC.
- /proc/net/dev lists lo, wlp7s0, eno1, eno2, virbr0, docker0, br-bc2d57ae738d, br-6bb7413a559e, vethcb88e24; virbr0/docker0/br-bc2d... show tx dropped=61; long names abut the colon (cat /proc/net/dev).
- sysfs speed values: veth/bridge-with-carrier = 10000, NO-CARRIER bridges = -1, wlp7s0 (down) read fails; /sys/class/net/eno1/statistics has 24 counters incl rx_dropped/tx_dropped/rx_missed_errors/rx_nohandler (ls).
- net.ipv4.ping_group_range = 0 2147483647, set by /usr/lib/sysctl.d/50-default.conf line 45 (sysctl + grep).
- Unprivileged SOCK_DGRAM/IPPROTO_ICMP echo to 192.168.100.1 succeeded from Python (14-byte reply, 1.41 ms); kernel rewrote the ICMP identifier (sent 1, reply 57270); SOCK_DGRAM/IPPROTO_ICMPV6 socket creation succeeded; SOCK_RAW ICMP and AF_PACKET both failed with EPERM (Errno 1).
- TCP connect to 192.168.100.1:53 succeeded in 3.45 ms unprivileged (Python socket.create_connection).
- /usr/bin/ping and /usr/bin/mtr-packet have cap_net_raw=ep (getcap); setcap/getcap present at /usr/sbin.
- Default IPv4 route: /proc/net/route eno1 dest 00000000 gateway 0164A8C0 metric 100 (= 192.168.100.1); ip -j route get 1.1.1.1 -> gateway 192.168.100.1 dev eno1 prefsrc 192.168.100.154; no IPv6 default route and no global IPv6 address (ip -j -6 route show default = []; ip -j -6 addr show scope global = []).
- DNS: /etc/resolv.conf is the systemd-resolved stub (nameserver 127.0.0.53, search mabeaman.com); resolvectl --json=short dns works; busctl get-property org.freedesktop.resolve1 ... Manager DNS = a(iiay) 1 entry: ifindex 3, AF_INET, 192.168.100.1; CurrentDNSServer empty.
- nsswitch hosts line: files mdns4_minimal [NOTFOUND=return] mymachines dns (grep /etc/nsswitch.conf).
- /proc/net/{tcp,tcp6,udp,udp6} are mode 0444 (ls -l) and contain 80/3/14/3 rows (wc -l); ss -tunaH counts 96 sockets.
- Scanning /proc/*/fd as the user: 628 processes, 143 readable, 482 unreadable, 6,360 fds, 990 socket inodes, 9.8 ms in Python; joining with /proc/net tables (103 entries) attributed 87 to a PID in 1.25 ms.
- ss -tunapH prints users:(("steam",pid=10144,fd=125)) only for own-user sockets (root-owned dnsmasq/resolved rows have no users field); ss -tunapHe adds ino:/uid:/cgroup:; ss -tinH state established returns tcp_info (rtt:10.44/0.595, minrtt, cwnd, delivery_rate) unprivileged; ss --help has no json option.
- Kernel config: CONFIG_INET_DIAG=m, CONFIG_INET_TCP_DIAG=m, CONFIG_INET_UDP_DIAG=m, CONFIG_PACKET=y, CONFIG_BPF_SYSCALL=y, CONFIG_DEBUG_INFO_BTF=y, CONFIG_CFG80211_WEXT=y, CONFIG_NF_CONNTRACK_PROCFS unset (/boot/config-7.0.0-30-generic).
- kernel.unprivileged_bpf_disabled = 2; /sys/kernel/btf/vmlinux present (7 MB); /usr/sbin/bpftool present; clang and bpf-linker absent; kernel.yama.ptrace_scope = 1.
- nf_conntrack module loaded (lsmod) but /proc/net/nf_conntrack does not exist and net.netfilter.nf_conntrack_acct = 0.
- Wi-Fi: wlp7s0 operstate down, /proc/net/wireless has header only, iw dev shows phy#0 wlp7s0 type managed, iw dev wlp7s0 link = Not connected; /sys/class/net/wlp7s0/{wireless,phy80211} exist; NetworkManager GetDeviceByIpIface wlp7s0 -> /org/freedesktop/NetworkManager/Devices/4 with Device.Wireless Bitrate=0, ActiveAccessPoint='/', Device.State=20 (busctl).
- System bus has org.freedesktop.NetworkManager (pid 2550), fi.w1.wpa_supplicant1 and org.freedesktop.resolve1 (busctl --system list).
- Installed CLI: ip/ss (iproute2 6.19.0), iw 6.17, nmcli 1.54.3, resolvectl, ping (iputils 20250605), mtr, tcpdump, ethtool; absent: iwconfig, traceroute, nethogs, iftop, bmon, bandwhich, sniffnet, gping, trippy (which).
- Packages: libpcap0.8t64 1.10.6, libnl-3-200 and libnl-genl-3-200 3.12.0 installed; libpcap-dev not installed (candidate 1.10.6-1ubuntu1) (dpkg -l, apt-cache policy). apt cache: iftop and nethogs depend on libpcap0.8t64; bmon depends on libnl-3-200 + libnl-route-3-200.
- /etc/services exists (365 lines) with entries like https 443/tcp and https 443/udp.
- ip -j -s link show eno1 returns stats64 JSON (rx bytes/packets/errors/dropped/multicast, tx bytes/packets/errors/dropped/carrier_errors/collisions).
- crates.io latest (cargo search today): pnet 0.35.0, pcap 2.5.0, neli 0.7.4, neli-wifi 0.6.1, wl-nl80211 0.7.0, netlink-packet-route 0.33.0, rtnetlink 0.23.0, netlink-packet-sock-diag 0.5.0, netlink-sys 0.9.0, if-addrs 0.15.0, nix 0.31.3, surge-ping 0.9.0 (rust-version 1.85), fastping-rs 0.2.4, dns-lookup 4.0.1, hickory-resolver 0.26.1, netstat2 0.11.2, listeners 0.6.1, bandwhich 0.23.1, sniffnet 1.5.1, trippy 0.13.0, trippy-core 0.13.0, gping 1.20.4, pinger 2.1.4, etherparse 0.21.0, socket2 0.6.5, aya 0.14.0, ureq 3.4.0, reqwest 0.13.4, public-ip 0.2.2, caps 0.5.6, procfs-core 0.18.0, network-interface 2.0.5, pnet_datalink 0.35.0.
- cargo info sysinfo: default features [component, disk, network, system, user]; procfs default features [chrono, flate2].
- sysinfo 0.39.6 docs: Networks::new()/new_with_refreshed_list()/list()/refresh(&mut self, remove_not_listed_interfaces: bool); NetworkData::received()/transmitted()/packets_*()/errors_on_*() are 'since the last refresh', total_* cumulative, plus mac_address(), ip_networks() -> &[IpNetwork{addr, prefix}], mtu(), operational_state() (docs.rs).
- sysinfo Linux source (src/unix/linux/network.rs): reads only rx_bytes,tx_bytes,rx_packets,tx_packets,rx_errors,tx_errors + mtu + operstate from sysfs, discovers new interfaces on every refresh, calls refresh_networks_addresses (getifaddrs) each refresh, no dropped counters, does not skip lo.
- procfs 0.18 docs: net::tcp()/tcp6()/udp()/udp6()/unix()/arp()/route()/dev_status()/snmp()/snmp6(); TcpNetEntry{local_address: SocketAddr, remote_address: SocketAddr, state: TcpState, rx_queue: u32, tx_queue: u32, uid: u32, inode: u64}; TcpState variants Established=1..NewSynRecv=12 with from_u8/to_u8; DeviceStatus 17 fields (name + 16 u64 counters); RouteEntry{iface, destination, gateway, flags: u16, refcnt, in_use, metrics: u32, mask, mtu, window, irtt}; Process::fd() -> ProcResult<FDsIter> yielding ProcResult<FDInfo{fd: i32, mode: u16, target: FDTarget}>; FDTarget::{Path(PathBuf), Socket(u64), Net(u64), Pipe(u64), AnonInode(String), MemFD(String), Other(String,u64), Unknown(String,String)}; all_processes() -> ProcResult<ProcessesIter> of Result<Process, ProcError>; Process::tcp()/tcp6()/udp()/udp6()/dev_status() read the per-process net namespace.
- bandwhich 0.23.1 Cargo.toml has no [lib] section; deps pnet 0.35.0, procfs 0.18.0 (linux), hickory-resolver 0.26.1, ratatui 0.30.0, tokio 1.52, resolv-conf 0.7.6; README setcap line: cap_sys_ptrace,cap_dac_read_search,cap_net_raw,cap_net_admin+ep; os/linux.rs builds inode->ProcessInfo via all_processes()/fd()/FDTarget::Socket then joins tcp()/tcp6()/udp()/udp6(); os/shared.rs uses datalink::interfaces()/channel() with read_timeout 1s, read_buffer_size 65536, keeps interfaces where is_up() && !ips.is_empty(); dns/client.rs uses a pending HashSet<IpAddr>, Arc<Mutex<IpTable>> cache, mpsc(1000) to a tokio thread; dns/resolver.rs calls reverse_lookup(ip), maps is_no_records_found() to the IP string.
- surge-ping 0.9 README/source: Config default sock_type_hint = Type::DGRAM, Client::new(&Config) -> io::Result<Self> tries the configured socket type then the opposite (DGRAM->RAW fallback), pinger(&self, host: IpAddr, ident: PingIdentifier) -> Pinger (async), ping(PingSequence, &[u8]) -> Result<(IcmpPacket, Duration), SurgeError>, replies demuxed by (IpAddr, Option<PingIdentifier>, PingSequence); README recommends sysctl ping_group_range or setcap cap_net_raw+ep.
- trippy: trippy.rs privileges guide says Linux needs setcap CAP_NET_RAW+p, unprivileged mode is macOS-only; trippy-privilege (gh api) uses caps::has_cap(None, CapSet::Permitted, CAP_NET_RAW) -> caps::raise(Effective) -> caps::clear(None, CapSet::Effective); trippy-core creates sockets with socket2::Socket::new(Domain::IPV4, Type::RAW|DGRAM, Some(Protocol::ICMPV4)).
- sniffnet wiki: Debian build deps libpcap-dev libasound2-dev libfontconfig1-dev libgtk-3-dev; run with setcap cap_net_raw,cap_net_admin=eip; nethogs README: libpcap + /proc, setcap cap_net_admin,cap_net_raw,cap_dac_read_search,cap_sys_ptrace+pe, libnethogs experimental; pinger 2.1.4 spawns the system ping and parses iputils output; pnet_datalink 0.35 default Linux backend is AF_PACKET with no external libs, Config{read_timeout, write_timeout, read_buffer_size, channel_type, promiscuous, linux_fanout}.
- neli-wifi 0.6.1 docs: Socket::connect(), get_interfaces_info(&mut self) -> Result<Vec<Interface>>, get_station_info(&mut self, interface_index: i32) -> Result<Vec<Station>>, get_bss_info; Interface{index: Option<i32>, ssid: Option<Vec<u8>>, mac: Option<Vec<u8>>, name: Option<Vec<u8>>, frequency/channel/power/phy: Option<u32>, device: Option<u64>}; Station{signal: Option<i8> dBm, average_signal: Option<i8>, rx_bitrate/tx_bitrate: Option<u32>, connected_time, beacon_loss, bssid, rx_packets, tx_packets, tx_failed, tx_retries, ht/vht/he/eht_mcs}; async feature with tokio. wl-nl80211 0.7.0: new_connection(), handle.station().dump(if_index).execute() stream, Nl80211RateInfo::Bitrate(u16)/Bitrate32(u32) documented as 100kb/s.
- netlink-packet-route 0.33 LinkAttribute variants include IfName, Mtu, Address, Broadcast, OperState(State), Carrier(u8), CarrierUpCount, CarrierDownCount, Stats(Stats), Stats64(Stats64 with rx/tx bytes, packets, errors, dropped, rx_missed_errors, rx_nohandler, multicast, collisions, ...). netlink-packet-sock-diag 0.5 inet::nlas::Nla::TcpInfo(Vec<u8>) is raw bytes (not parsed); example dump_ipv4.rs uses Socket::new(NETLINK_SOCK_DIAG), NLM_F_REQUEST|NLM_F_DUMP, SockDiagMessage::InetRequest/InetResponse.
- if-addrs (source, master): get_if_addrs() -> io::Result<Vec<Interface{name, addr: IfAddr, index: Option<u32>, oper_status, is_p2p}>>, Ifv4Addr{ip, netmask, prefixlen, broadcast: Option}, libc getifaddrs backend; nix 0.31.3 ifaddrs module requires feature net. hickory-resolver 0.26.1: default features system-config + tokio; Resolver::builder_tokio(), reverse_lookup(impl IntoName), txt_lookup, lookup_ip. dns-lookup 4.0.1: lookup_addr(&IpAddr) -> Result<String, LookupError> (blocking getnameinfo).
- ratatui 0.30.2: Sparkline::data accepts IntoIterator<Item: Into<SparklineBar>> (u64, Option<u64>, SparklineBar), max(u64), direction(RenderDirection, default LeftToRight), absent_value_style/absent_value_symbol, bar_set(THREE_LEVELS|NINE_LEVELS); Dataset::data(&[(f64,f64)]), marker(Marker::{Dot,Block,Bar,Braille,HalfBlock}), graph_type(GraphType::{Scatter,Line,Bar,Area}), fill_to_y(f64).
- Kernel docs: ping_group_range default '1 0' = nobody may create ping sockets; unprivileged_bpf_disabled 2 = unprivileged bpf() disabled but re-enableable, bpf() without CAP_SYS_ADMIN/CAP_BPF returns EPERM; capabilities(7): CAP_NET_RAW = RAW and PACKET sockets, CAP_BPF added in 5.8, CAP_SYS_PTRACE covers inspecting other processes' /proc; /proc/net/wireless line format '%6s: %04x  %3d%c  %3d%c  %3d%c  %6d...' from net/wireless/wext-proc.c with '.' = updated; proc_net_tcp.txt field meanings (hex addresses, st, tx/rx queue, uid, inode).
- BCC tcptop attaches kprobes to tcp_sendmsg/tcp_recvmsg keyed by pid/comm/laddr/lport/daddr/dport, TCP only. ipify docs: https://api.ipify.org, api6.ipify.org, api64.ipify.org, ?format=json, 'no limit'.

## Open questions

- Privileged helper strategy: separate setcap'd helper binary over a unix socket (recommended), setcap on the main opstui binary, or a systemd unit with AmbientCapabilities (astral-watch precedent)? Also whether it should live in the same repo/workspace.
- Is per-process bandwidth a must-have for v1.0, or acceptable as a later arc (possibly eBPF/aya once a nightly + bpf-linker toolchain is acceptable in CI)?
- Should the connection table use /proc/net/tcp (simple) or netlink sock_diag (unprivileged, adds per-connection RTT/throughput via raw tcp_info parsing) from the start?
- Which interfaces should be visible by default on torch: only eno1 (+ wlp7s0 when up), or also wg*/tun* VPNs and libvirt bridges when they carry traffic?
- Probe policy: default targets (gateway, 1.1.1.1, 8.8.8.8), 1 Hz cadence, and whether ICMP probes should be on by default or require opt-in like the public-IP lookup; IPv6 probes are moot until a global v6 address exists.
- Reverse DNS default: on, off, or on for public IPs only (it leaks remote endpoints to 192.168.100.1's resolver)?
- Wi-Fi source preference: neli-wifi (dBm, bitrate, MCS; pulls neli 0.6) vs NetworkManager D-Bus via zbus (percent strength, push signals) - can the Wi-Fi arc be validated by temporarily connecting wlp7s0?
- Do you want a 'trace' (traceroute) mode via trippy-core later, given it needs cap_net_raw?
- Sampling cadence: 1 Hz only, or 250 ms counters with smoothing for the retrowave sparklines?
- Should the component also show per-interface carrier flap counts and kernel drop deltas prominently (useful on the 2.5GbE r8169 which currently links at 1G)?

## Sources

- https://docs.rs/sysinfo/0.39.6/sysinfo/struct.Networks.html
- https://docs.rs/sysinfo/0.39.6/sysinfo/struct.NetworkData.html
- https://docs.rs/sysinfo/0.39.6/sysinfo/struct.IpNetwork.html
- https://docs.rs/sysinfo/0.39.6/sysinfo/enum.InterfaceOperationalState.html
- https://raw.githubusercontent.com/GuillaumeGomez/sysinfo/master/src/unix/linux/network.rs
- https://docs.rs/procfs/0.18.0/procfs/net/index.html
- https://docs.rs/procfs/0.18.0/procfs/process/struct.Process.html
- https://docs.rs/procfs/0.18.0/procfs/process/enum.FDTarget.html
- https://docs.rs/procfs/0.18.0/procfs/process/struct.FDInfo.html
- https://docs.rs/procfs/0.18.0/procfs/process/struct.FDsIter.html
- https://docs.rs/procfs/0.18.0/procfs/process/fn.all_processes.html
- https://docs.rs/procfs-core/0.18.0/procfs_core/net/struct.TcpNetEntry.html
- https://docs.rs/procfs-core/0.18.0/procfs_core/net/enum.TcpState.html
- https://docs.rs/procfs-core/0.18.0/procfs_core/net/struct.DeviceStatus.html
- https://docs.rs/procfs-core/0.18.0/procfs_core/net/struct.RouteEntry.html
- https://github.com/imsnif/bandwhich/blob/main/README.md
- https://raw.githubusercontent.com/imsnif/bandwhich/main/Cargo.toml
- https://raw.githubusercontent.com/imsnif/bandwhich/main/src/os/linux.rs
- https://raw.githubusercontent.com/imsnif/bandwhich/main/src/os/shared.rs
- https://raw.githubusercontent.com/imsnif/bandwhich/main/src/network/dns/client.rs
- https://raw.githubusercontent.com/imsnif/bandwhich/main/src/network/dns/resolver.rs
- https://raw.githubusercontent.com/kolapapa/surge-ping/main/README.md
- https://raw.githubusercontent.com/kolapapa/surge-ping/main/src/config.rs
- https://raw.githubusercontent.com/kolapapa/surge-ping/main/src/client.rs
- https://trippy.rs/guides/privileges/
- https://github.com/fujiapple852/trippy (crates/trippy-privilege/src/lib.rs, crates/trippy-core/src/net/platform/unix.rs via gh api)
- https://github.com/GyulyVGC/sniffnet/wiki/Required-dependencies
- https://github.com/raboof/nethogs/blob/main/README.md
- https://github.com/tgraf/bmon/blob/master/README.md
- https://docs.rs/pinger/2.1.4/pinger/
- https://docs.rs/pnet_datalink/0.35.0/pnet_datalink/
- https://docs.rs/neli-wifi/0.6.1/neli_wifi/ (Socket, Interface, Station pages)
- https://docs.rs/wl-nl80211/0.7.0/wl_nl80211/ and enum.Nl80211RateInfo.html
- https://raw.githubusercontent.com/rust-netlink/wl-nl80211/main/examples/dump_nl80211_station.rs
- https://docs.rs/netlink-packet-route/0.33.0/netlink_packet_route/link/struct.Stats64.html
- https://docs.rs/netlink-packet-route/0.33.0/netlink_packet_route/link/enum.LinkAttribute.html
- https://docs.rs/netlink-packet-sock-diag/0.5.0/netlink_packet_sock_diag/inet/index.html and inet/nlas/enum.Nla.html
- https://raw.githubusercontent.com/rust-netlink/netlink-packet-sock-diag/main/examples/dump_ipv4.rs
- https://docs.rs/nix/0.31.3/nix/ifaddrs/index.html
- https://raw.githubusercontent.com/messense/if-addrs/master/src/lib.rs
- https://docs.rs/hickory-resolver/0.26.1/hickory_resolver/struct.Resolver.html
- https://docs.rs/crate/hickory-resolver/0.26.1/features
- https://docs.rs/dns-lookup/4.0.1/dns_lookup/fn.lookup_addr.html
- https://docs.rs/caps/0.5.6/caps/
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Sparkline.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Dataset.html
- https://docs.rs/public-ip/0.2.2/public_ip/
- https://www.ipify.org/
- https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.AccessPoint.html
- https://www.kernel.org/doc/Documentation/networking/proc_net_tcp.txt
- https://www.kernel.org/doc/html/latest/networking/ip-sysctl.html
- https://www.kernel.org/doc/html/latest/admin-guide/sysctl/kernel.html
- https://raw.githubusercontent.com/torvalds/linux/master/net/wireless/wext-proc.c
- https://man7.org/linux/man-pages/man7/capabilities.7.html
- https://github.com/iovisor/bcc/blob/master/tools/tcptop.py
- Local read-only checks on host torch (2026-08-30): /proc/net/{dev,route,ipv6_route,if_inet6,wireless,tcp,udp,snmp,sockstat,arp}, /sys/class/net/*, sysctl, busctl, ip -j, ss, iw, nmcli, resolvectl, ethtool, getcap, dpkg/apt-cache, /boot/config-7.0.0-30-generic, cargo search/info, Python socket/timing experiments
