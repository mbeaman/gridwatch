# Research digests

Ten deep-dives produced 2026-08-30 against torch (read-only) and the crate docs. Each has a summary, recommendations, crates (with system deps + confidence), risks, *verified facts* (how each was checked), open questions and sources.

- [ratatui-030-ecosystem](ratatui-030-ecosystem.md) — ratatui 0.30 and its ecosystem as the rendering foundation for opsTui (0.29→0.30 changes, rendering primitives
- [audio-capture-and-fft](audio-capture-and-fft.md) — Playback-audio capture on PipeWire 1.6 (no dev headers) + spectrum/oscilloscope/VU DSP and ratatui rendering f
- [winamp-mpris](winamp-mpris.md) — Winamp-inspired MPRIS2 "now playing" component for opsTui: MPRIS2/zbus 5 client design, Firefox quirks, Winamp
- [htop-parity](htop-parity.md) — Reproducing htop 3.4.1 as an opsTui grid component (feature inventory, Linux data sources in Rust, size-class 
- [nvtop-parity](nvtop-parity.md) — Reproducing nvtop 3.2 for the RTX 5090 via NVML (nvml-wrapper 0.12.1), plus GPU-Z-style static specs from the 
- [network-monitoring](network-monitoring.md) — Network-monitoring component for opsTui: data sources (interface, connection, per-process, latency), privilege
- [astral-watch-and-sensors](astral-watch-and-sensors.md) — Integrating astral-watch (per-pin 12V-2x6 amperage) as an opsTui component, plus a general sensors source (hwm
- [theme-system](theme-system.md) — opsTui theme system: first-class "retrowave" / "modern" / phosphor / etc. looks in a ratatui 0.30 truecolor TU
- [grid-layout-engine](grid-layout-engine.md) — opsTui grid layout engine: prior art, model choice, size classes, pages/zoom/focus/edit mode, TOML schema + ho
- [prior-art-dashboards](prior-art-dashboards.md) — Prior-art survey for opsTui: what existing system/GPU/network/audio/player TUIs do well, what to avoid, and wh
