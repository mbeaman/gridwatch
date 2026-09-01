# Changelog

All notable changes to gridwatch. One minor version per arc (see `docs/ROADMAP.md`).

## [Unreleased]
### Added
- Arc 1a — the core seam: six-crate workspace (`store ← ui ← components ← app ← bin`, `store ← sources ← app`); typed single-writer `Store` with three channels and drain order input→control→data≤3ms; `View` tree + `Component`/`Renderer`/`Theme` contracts with Oklab gradient LUTs and the ColorMode ladder; 12×6 grid solver with configured/dense/stack modes and 2-cell hysteresis; source supervisor (catch_unwind, backoff, `Demand` zero-poll timers); app shell (render cache, heartbeat, panic containment, focus/zoom/pause/theme keys, F12 HUD); `gridwatch run [--demo]`, `shot` (byte-deterministic, D41), `config check|default`, `doctor`; themes retrowave/modern/mono; clock + sources tiles; schemas + snapshot testkit + perf measure script. First measured P-rows: idle demo 0.0% CPU, 4 wake/s, ~0 KB/s, 10 MB RSS.
- Planning baseline: architecture, roadmap, performance ceilings, model strategy, review gates, arc 1a/1b briefs, research digests and design-review provenance (decisions D01–D40).
