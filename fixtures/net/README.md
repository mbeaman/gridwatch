# Network fixtures (§12, brief arc 7)

Copied from torch on 2026-09-02 — every file is a read of `/proc` or
`/sys`, nothing was configured or changed:

| path | what it pins |
|---|---|
| `proc/net/dev` | the sixteen counters per interface, and the name that butts against the colon (`br-6bb7413a559e:      84`) — splitting on whitespace would swallow a counter |
| `proc/net/route` | the default route as little-endian hex (`0164A8C0` = 192.168.100.1) |
| `proc/net/tcp`, `tcp6`, `udp` | the connection tables' columns, states and hex addresses (the first rows only) |
| `sys/class/net/<if>/…` | `eno1` up at 1000 Mb/s, `wlp7s0` down with an **unreadable** `speed` and a `wireless` directory, a `br-…` bridge with no carrier, and `lo` |

The Wi-Fi radio is down on torch, so the connected path (SSID, dBm,
bitrate) has no fixture and its live row is owed to Matt.
