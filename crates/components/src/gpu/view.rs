//! The gpu tier builders (§8): nvtop's header lines, the gauges, the
//! ten-minute chart band and the process table, each laid out from the real
//! inner rect with saturating arithmetic. Nothing here reads a device, names a
//! colour or picks a glyph.

use std::borrow::Cow;

use gridwatch_store::keys::gpu::{self, GpuInfo, Throttle};
use gridwatch_store::{Agg, Key, Store};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{Bounds, Constraint, Dir, MarkerHint, Series, Span, View};
use ratatui_core::layout::Rect;

use super::format as fmt;
use super::{
    CHART_SPAN, Gpu, HEADER_ROWS, SERIES, TIER_BADGE, TIER_CHARTS, TIER_GAUGES, TIER_HEADER,
    TIER_PROCS,
};

/// The GPU-Z column's width when the chart band is at least this wide.
const SPEC_COLUMN_AT: u16 = 100;
const SPEC_COLUMN_W: u16 = 24;

const DEV: u16 = 0;

fn scalar(store: &Store, key: &Key<f64>) -> Option<f64> {
    store.last(&key.idx(DEV)).map(|(_, v)| v)
}

fn info(store: &Store) -> Option<GpuInfo> {
    store.record(&gpu::INFO.idx(DEV)).map(|(_, i)| i.clone())
}

fn throttle(store: &Store) -> Throttle {
    store
        .record(&gpu::THROTTLE.idx(DEV))
        .map(|(_, t)| *t)
        .unwrap_or_default()
}

fn muted(s: impl Into<Cow<'static, str>>) -> Span {
    Span::new(Role::TextMuted, s)
}

fn ghost(s: impl Into<Cow<'static, str>>) -> Span {
    Span::new(Role::TextGhost, s)
}

fn value(s: impl Into<Cow<'static, str>>) -> Span {
    Span::new(Role::Text, s)
}

/// nvtop's temperature colour: green below slowdown − 5, yellow near it, red
/// at or above — as roles.
fn temp_role(temp: Option<f64>, slowdown: Option<f64>) -> Role {
    match (temp, slowdown) {
        (Some(t), Some(s)) if t >= s => Role::Crit,
        (Some(t), Some(s)) if t >= s - 5.0 => Role::Warn,
        (Some(_), _) => Role::Ok,
        (None, _) => Role::TextGhost,
    }
}

fn frac(v: Option<f64>, max: Option<f64>) -> f32 {
    match (v, max) {
        (Some(v), Some(m)) if m > 0.0 => (v / m).clamp(0.0, 1.0) as f32,
        _ => 0.0,
    }
}

/// The util sparkline over the run's age, capped at the chart span.
fn util_spark(cx: &RenderCx<'_>, buckets: u16) -> View {
    let mut out = Vec::new();
    let span = std::time::Duration::from_nanos(cx.now.0)
        .min(CHART_SPAN)
        .max(std::time::Duration::from_secs(1));
    cx.store.resample(
        &gpu::UTIL_PCT.idx(DEV),
        span,
        buckets.max(1) as usize,
        Agg::Avg,
        &mut out,
    );
    View::Sparkline {
        series: out.into_iter().map(|v| v.map(|v| v as f32)).collect(),
        gradient: GradientId::Load,
        max: Some(100.0),
    }
}

/// The throttle chip text, if any reason is limiting the clocks.
fn throttle_chip(store: &Store) -> Option<String> {
    let t = throttle(store);
    if t.is_limiting() {
        Some(t.labels().join(" "))
    } else {
        None
    }
}

// ---------------------------------------------------------------- badge

fn badge(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let util = scalar(store, &gpu::UTIL_PCT);
    let temp = scalar(store, &gpu::TEMP_C);
    let slowdown = scalar(store, &gpu::TEMP_SLOWDOWN_C);
    let line1 = vec![
        muted("GPU "),
        value(fmt::pct(util).trim_start().to_string()),
    ];
    let mut line2 = vec![Span::new(temp_role(temp, slowdown), fmt::celsius(temp))];
    if let Some(chip) = throttle_chip(store) {
        line2.push(Span::new(Role::Warn, format!(" {chip}")));
    } else if let Some(p) = scalar(store, &gpu::PSTATE)
        && p < 16.0
    {
        line2.push(ghost(format!(" P{}", p as u8)));
    }
    let mut children = vec![
        (Constraint::Len(1), View::Text(vec![line1])),
        (Constraint::Len(1), View::Text(vec![line2])),
    ];
    if cx.inner.height >= 3 {
        children.push((Constraint::Fill(1), util_spark(cx, cx.inner.width)));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

// ---------------------------------------------------------------- gauges

fn gauge(
    label: &'static str,
    v: Option<f64>,
    max: Option<f64>,
    g: GradientId,
    text: String,
) -> View {
    View::Gauge {
        label: label.into(),
        value: frac(v, max),
        gradient: g,
        text: Some(text.into()),
    }
}

fn gauges_block(cx: &RenderCx<'_>) -> Vec<(Constraint, View)> {
    let store = cx.store;
    let util = scalar(store, &gpu::UTIL_PCT);
    let used = scalar(store, &gpu::VRAM_USED_B);
    let total = scalar(store, &gpu::VRAM_TOTAL_B);
    let memctl = scalar(store, &gpu::MEMCTL_PCT);
    let vram_text = match (used, total) {
        (Some(u), Some(t)) => format!(
            "{}/{} {}",
            fmt::human_bytes(u),
            fmt::human_bytes(t),
            fmt::pct(Some(u / t * 100.0)).trim_start()
        ),
        _ => "—".into(),
    };
    vec![
        (
            Constraint::Len(1),
            gauge(
                "GPU   ",
                util,
                Some(100.0),
                GradientId::Load,
                fmt::pct(util).trim_start().to_string(),
            ),
        ),
        (
            Constraint::Len(1),
            gauge("VRAM  ", used, total, GradientId::Mem, vram_text),
        ),
        (
            Constraint::Len(1),
            gauge(
                "MEMCTL",
                memctl,
                Some(100.0),
                GradientId::Mem,
                fmt::pct(memctl).trim_start().to_string(),
            ),
        ),
    ]
}

/// `2790MHz 400/600W 45°C 30%` — the gauges tier's compressed line 2.
fn clocks_line(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let temp = scalar(store, &gpu::TEMP_C);
    let slowdown = scalar(store, &gpu::TEMP_SLOWDOWN_C);
    let fan = store
        .last(&gpu::FAN_PCT.named(&gpu::fan_label(DEV, 0)))
        .map(|(_, v)| v);
    View::Text(vec![vec![
        value(fmt::mhz(scalar(store, &gpu::CLOCK_GFX_MHZ))),
        ghost(" "),
        value(fmt::power(
            scalar(store, &gpu::POWER_W),
            scalar(store, &gpu::POWER_LIMIT_W),
        )),
        ghost(" "),
        Span::new(temp_role(temp, slowdown), fmt::celsius(temp)),
        ghost(" "),
        muted("fan "),
        value(fmt::pct(fan).trim_start().to_string()),
    ]])
}

fn status_line(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let mut line = Vec::new();
    // A source that is not `Ok` says why, in the tile (review: `device = 7`
    // left the tile "waiting" with the reason only in the sources tile).
    let st = store.status(gpu::SOURCE);
    if !matches!(st.state, gridwatch_store::SourceState::Ok)
        && let Some(reason) = &st.reason
    {
        return View::Text(vec![vec![Span::new(
            Role::Warn,
            format!("gpu source: {reason}"),
        )]]);
    }
    if let Some(p) = scalar(store, &gpu::PSTATE) {
        if p < 16.0 {
            line.push(muted(format!("P{}", p as u8)));
        } else {
            line.push(ghost("P?"));
        }
    }
    match throttle_chip(store) {
        Some(chip) => {
            line.push(ghost(" "));
            line.push(Span::bold(Role::Warn, chip));
        }
        None if scalar(store, &gpu::UTIL_PCT).is_some() => {
            line.push(ghost(" no throttling"));
        }
        None => line.push(ghost("waiting for the gpu source…")),
    }
    View::Text(vec![line])
}

fn gauges(cx: &RenderCx<'_>) -> View {
    let mut children = gauges_block(cx);
    children.push((Constraint::Len(1), clocks_line(cx)));
    children.push((Constraint::Len(1), status_line(cx)));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

// ---------------------------------------------------------------- header

/// Line 1: `Device 0 [name]  PCIe GEN 5@16x RX: 12 MiB/s TX: 3 MiB/s`. The
/// PCIe half is built first and the name is cut to what remains, so the tier's
/// signature survives at its minimum width.
fn device_line(cx: &RenderCx<'_>, width: u16) -> View {
    let store = cx.store;
    let generation = scalar(store, &gpu::PCIE_GEN);
    let lanes = scalar(store, &gpu::PCIE_WIDTH);
    let link = match (generation, lanes) {
        (Some(g), Some(w)) => format!("PCIe GEN {}@{:>2}x", g as u8, w as u8),
        _ => "PCIe —".into(),
    };
    let rx = fmt::rate(scalar(store, &gpu::PCIE_RX_BPS));
    let tx = fmt::rate(scalar(store, &gpu::PCIE_TX_BPS));
    let right = format!("{link} RX: {rx} TX: {tx}");
    let right_w = right.chars().count() as u16;
    let name = info(store)
        .map(|i| i.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "—".into());
    let left_budget = usize::from(width.saturating_sub(right_w + 2));
    let left_full = format!("Device {DEV} [{name}]");
    let left = if left_full.chars().count() <= left_budget {
        left_full
    } else {
        // Drop the vendor and family prefixes first (`RTX 5090`, not
        // `Ge…`), then truncate with an ellipsis.
        let short = name
            .trim_start_matches("NVIDIA ")
            .trim_start_matches("GeForce ")
            .to_string();
        let s = format!("Device {DEV} [{short}]");
        if s.chars().count() <= left_budget {
            s
        } else {
            // Keep the closing bracket: `Device 0 [RTX 50…]`.
            let prefix = format!("Device {DEV} [");
            let keep = left_budget.saturating_sub(prefix.chars().count() + 2);
            let cut: String = short.chars().take(keep).collect();
            if keep > 0 {
                format!("{prefix}{cut}…]")
            } else {
                format!("Device {DEV}")
            }
        }
    };
    let pad = usize::from(width).saturating_sub(left.chars().count() + usize::from(right_w));
    View::Text(vec![vec![
        value(left),
        ghost(" ".repeat(pad.max(1))),
        muted(right),
    ]])
}

/// Line 2: `GPU 2790MHz MEM 14001MHz TEMP 45°C FAN 30% POW 400/600W`.
fn clocks_header_line(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let temp = scalar(store, &gpu::TEMP_C);
    let slowdown = scalar(store, &gpu::TEMP_SLOWDOWN_C);
    let fans: Vec<f64> = store
        .labels(gpu::FAN_PCT.id.name)
        .filter_map(|l| {
            store
                .last(&Key::<f64>::new(gpu::FAN_PCT.id.name).named(match l {
                    gridwatch_store::Label::Name(n) => n,
                    _ => return None,
                }))
                .map(|(_, v)| v)
        })
        .collect();
    let fan = if fans.is_empty() {
        None
    } else {
        Some(fans.iter().copied().fold(0.0, f64::max))
    };
    View::Text(vec![vec![
        muted("GPU "),
        value(fmt::mhz(scalar(store, &gpu::CLOCK_GFX_MHZ))),
        muted(" MEM "),
        value(fmt::mhz(scalar(store, &gpu::CLOCK_MEM_MHZ))),
        muted(" TEMP "),
        Span::new(temp_role(temp, slowdown), fmt::celsius(temp)),
        muted(" FAN "),
        value(fmt::pct(fan).trim_start().to_string()),
        muted(" POW "),
        value(fmt::power(
            scalar(store, &gpu::POWER_W),
            scalar(store, &gpu::POWER_LIMIT_W),
        )),
    ]])
}

/// Two bars side by side (nvtop's line 3 and 4).
fn bar_pair(a: View, b: View) -> View {
    View::Stack {
        dir: Dir::H,
        children: vec![
            (Constraint::Fill(1), a),
            (Constraint::Len(2), View::Empty),
            (Constraint::Fill(1), b),
        ],
    }
}

fn power_trace(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let series: Vec<Option<f32>> = match store.vector(&gpu::POWER_TRACE.idx(DEV)) {
        Some((_, v)) => v.iter().map(|w| Some(*w)).collect(),
        None => Vec::new(),
    };
    let limit = scalar(store, &gpu::POWER_LIMIT_W).map(|l| l as f32);
    if series.is_empty() {
        return View::Text(vec![vec![ghost("POW 20 ms trace: waiting…")]]);
    }
    View::Sparkline {
        series,
        gradient: GradientId::Power,
        max: limit,
    }
}

/// nvtop's header (8 rows): device/PCIe, clocks, GPU+MEM bars, ENC+DEC bars
/// (or the P-state/throttle/fan line once they hide), the 20 ms power trace.
fn header_block(g: &Gpu, cx: &RenderCx<'_>, width: u16) -> Vec<(Constraint, View)> {
    let store = cx.store;
    let util = scalar(store, &gpu::UTIL_PCT);
    let used = scalar(store, &gpu::VRAM_USED_B);
    let total = scalar(store, &gpu::VRAM_TOTAL_B);
    let mem_pct = match (used, total) {
        (Some(u), Some(t)) if t > 0.0 => Some(u / t * 100.0),
        _ => None,
    };
    let gpu_bar = gauge(
        "GPU",
        util,
        Some(100.0),
        GradientId::Load,
        fmt::pct(util).trim_start().to_string(),
    );
    // nvtop's MEM bar is VRAM occupancy, not memory-controller load.
    let mem_bar = gauge(
        "MEM",
        used,
        total,
        GradientId::Mem,
        fmt::pct(mem_pct).trim_start().to_string(),
    );
    let enc = scalar(store, &gpu::ENC_PCT);
    let dec = scalar(store, &gpu::DEC_PCT);
    let line4 = if g.encdec_visible_at(cx.now) && (enc.is_some() || dec.is_some()) {
        bar_pair(
            gauge(
                "ENC",
                enc,
                Some(100.0),
                GradientId::Load,
                fmt::pct(enc).trim_start().to_string(),
            ),
            gauge(
                "DEC",
                dec,
                Some(100.0),
                GradientId::Load,
                fmt::pct(dec).trim_start().to_string(),
            ),
        )
    } else {
        status_line(cx)
    };
    vec![
        (Constraint::Len(1), device_line(cx, width)),
        (Constraint::Len(1), clocks_header_line(cx)),
        (Constraint::Len(1), bar_pair(gpu_bar, mem_bar)),
        (Constraint::Len(1), line4),
        (
            Constraint::Len(1),
            View::Text(vec![vec![muted("POW "), ghost("20 ms")]]),
        ),
        (Constraint::Len(3), power_trace(cx)),
    ]
}

fn header(g: &Gpu, cx: &RenderCx<'_>) -> View {
    View::Stack {
        dir: Dir::V,
        children: header_block(g, cx, cx.inner.width),
    }
}

// ---------------------------------------------------------------- charts

/// The six plottable series as percentages of their ceiling (digest §1).
fn series_points(
    store: &Store,
    name: &str,
    span: std::time::Duration,
    buckets: usize,
    reverse: bool,
) -> (Vec<(f64, f64)>, Option<f64>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    // Device-labelled keys, owned: a `&CONST` is a temporary on rustc 1.88
    // (the MSRV), only 1.95 promotes it.
    let key = |k: &Key<f64>| k.idx(DEV);
    type Ratio = (Option<Key<f64>>, Option<(Key<f64>, f64)>);
    let (num, den): Ratio = match name {
        "util" => (Some(key(&gpu::UTIL_PCT)), None),
        "vram" => (
            Some(key(&gpu::VRAM_USED_B)),
            Some((key(&gpu::VRAM_TOTAL_B), 100.0)),
        ),
        "temp" => (Some(key(&gpu::TEMP_C)), None),
        "power" => (
            Some(key(&gpu::POWER_W)),
            Some((key(&gpu::POWER_LIMIT_W), 100.0)),
        ),
        "clock" => (
            Some(key(&gpu::CLOCK_GFX_MHZ)),
            Some((key(&gpu::CLOCK_GFX_MAX_MHZ), 100.0)),
        ),
        _ => (None, None),
    };
    let mut points = Vec::with_capacity(buckets);
    let mut last = None;
    let place = |i: usize| -> f64 {
        if reverse {
            (buckets - 1 - i) as f64
        } else {
            i as f64
        }
    };
    if name == "load" {
        // effective load = util × power / limit, capped at 100 (nvtop).
        let mut c = Vec::new();
        store.resample(&key(&gpu::UTIL_PCT), span, buckets, Agg::Avg, &mut a);
        store.resample(&key(&gpu::POWER_W), span, buckets, Agg::Avg, &mut b);
        store.resample(&key(&gpu::POWER_LIMIT_W), span, buckets, Agg::Last, &mut c);
        for i in 0..buckets {
            if let (Some(u), Some(p), Some(l)) = (a[i], b[i], c[i])
                && l > 0.0
            {
                let v = effective_load(u, p, l);
                points.push((place(i), v));
                last = Some(v);
            }
        }
        return (points, last);
    }
    let Some(num) = num else {
        return (points, None);
    };
    store.resample(&num, span, buckets, Agg::Avg, &mut a);
    if let Some((d, scale)) = den {
        store.resample(&d, span, buckets, Agg::Last, &mut b);
        // A ceiling published once per generation (`clock_gfx_max_mhz`) ages
        // out of the window: carry the latest value into every empty bucket
        // (review: the `clock` series was empty after ten minutes).
        if let Some((_, latest)) = store.last(&d) {
            let mut carry = None;
            for slot in b.iter_mut() {
                match slot {
                    Some(v) => carry = Some(*v),
                    None => *slot = Some(carry.unwrap_or(latest)),
                }
            }
        }
        for i in 0..buckets {
            if let (Some(n), Some(dv)) = (a[i], b[i])
                && dv > 0.0
            {
                let v = (n / dv * scale).min(100.0);
                points.push((place(i), v));
                last = Some(v);
            }
        }
    } else {
        for (i, v) in a.iter().enumerate() {
            if let Some(v) = v {
                points.push((place(i), v.min(100.0)));
                last = Some(*v);
            }
        }
    }
    (points, last)
}

fn series_gradient(name: &str) -> GradientId {
    match name {
        "util" => GradientId::Load,
        "vram" => GradientId::Mem,
        "temp" => GradientId::Temp,
        "power" => GradientId::Power,
        "clock" => GradientId::Title,
        _ => GradientId::Audio,
    }
}

/// The chart band: a legend line (series · last value, the window, `⟵` when
/// reversed) over the braille chart of every enabled series, 0–100 %.
fn chart_band(g: &Gpu, cx: &RenderCx<'_>, width: u16, rows: u16) -> View {
    let store = cx.store;
    let buckets = usize::from(width.max(1));
    let span = std::time::Duration::from_nanos(cx.now.0)
        .min(CHART_SPAN)
        .max(std::time::Duration::from_secs(1));
    let mut legend: Vec<Span> = Vec::new();
    let mut series = Vec::new();
    for (i, name) in SERIES.iter().enumerate() {
        if !g.series_on()[i] {
            continue;
        }
        let (points, last) = series_points(store, name, span, buckets, g.reversed());
        if !legend.is_empty() {
            legend.push(ghost("  "));
        }
        legend.push(muted(format!("{}:{name} ", i + 1)));
        legend.push(value(match last {
            Some(v) if *name == "temp" => format!("{}°", v.round() as i64),
            Some(v) => format!("{}%", v.round() as i64),
            None => "—".into(),
        }));
        series.push(Series {
            label: Cow::Borrowed(name),
            gradient: series_gradient(name),
            data: points,
        });
    }
    if legend.is_empty() {
        legend.push(ghost("no series selected (1–6)"));
    }
    let mins = span.as_secs() / 60;
    let window = if g.reversed() {
        format!("  ⟵ {mins}m")
    } else {
        format!("  {mins}m ⟶")
    };
    legend.push(ghost(window));
    let chart = if series.iter().all(|s| s.data.is_empty()) {
        View::Text(vec![vec![ghost("chart: waiting for history…")]])
    } else {
        View::Chart {
            series,
            bounds: Bounds {
                x: (0.0, (buckets.saturating_sub(1)).max(1) as f64),
                y: (0.0, 100.0),
            },
            marker: MarkerHint::Auto,
        }
    };
    let _ = rows;
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![legend])),
            (Constraint::Fill(1), chart),
        ],
    }
}

/// The GPU-Z column (§8, digest §4): the spec row plus what NVML confirmed.
fn spec_column(cx: &RenderCx<'_>) -> View {
    let Some(i) = info(cx.store) else {
        return View::Text(vec![vec![ghost("spec: —")]]);
    };
    let mut rows: Vec<(
        Cow<'static, str>,
        Vec<Span>,
        Option<gridwatch_store::Severity>,
    )> = Vec::new();
    let mut kv = |k: &'static str, v: String| rows.push((k.into(), vec![value(v)], None));
    if !i.arch.is_empty() {
        kv("arch", i.arch.clone());
    }
    if let Some(c) = i.cores {
        kv("cores", c.to_string());
    }
    if let Some(s) = &i.spec {
        kv("SMs", s.sms.to_string());
        kv("TMU/ROP", format!("{}/{}", s.tmus, s.rops));
        kv("RT/tensor", format!("{}/{}", s.rt_cores, s.tensor_cores));
        kv("L2", format!("{} MB", s.l2_mb));
        kv("base/boost", format!("{}/{}MHz", s.base_mhz, s.boost_mhz));
        kv("memory", format!("{} Gbps", s.mem_gbps));
        kv("bandwidth", format!("{} GB/s", s.bandwidth_gbs));
        kv("TDP", format!("{} W", s.tdp_w));
        kv("die", format!("{} mm²", s.die_mm2));
        kv("transistors", format!("{} B", s.transistors_b));
        kv("launch", s.launch.to_string());
    }
    if let Some(b) = i.bus_width {
        kv("bus", format!("{b}-bit"));
    }
    if !i.driver.is_empty() {
        kv("driver", i.driver.clone());
    }
    if !i.vbios.is_empty() {
        kv("vbios", i.vbios.clone());
    }
    if i.spec_mismatch {
        rows.push((
            "spec".into(),
            vec![Span::new(Role::Warn, "row disagrees with NVML")],
            None,
        ));
    }
    View::KeyValue(rows)
}

fn charts(g: &Gpu, cx: &RenderCx<'_>) -> View {
    let band = g.band_rows(TIER_CHARTS, cx.inner.height);
    let width = cx.inner.width;
    let with_spec = g.options().spec_column && width >= SPEC_COLUMN_AT;
    let chart_w = if with_spec {
        width.saturating_sub(SPEC_COLUMN_W + 1)
    } else {
        width
    };
    let band_view = if with_spec {
        View::Stack {
            dir: Dir::H,
            children: vec![
                (Constraint::Fill(1), chart_band(g, cx, chart_w, band)),
                (Constraint::Len(1), View::Empty),
                (Constraint::Len(SPEC_COLUMN_W), spec_column(cx)),
            ],
        }
    } else {
        chart_band(g, cx, chart_w, band)
    };
    let mut children = header_block(g, cx, width);
    children.push((Constraint::Fill(1), band_view));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

// ---------------------------------------------------------------- procs

fn table(g: &Gpu, cx: &RenderCx<'_>) -> View {
    let height = cx.inner.height;
    let band = g.band_rows(cx.tier, height);
    let body_rows = g.body_rows(cx.tier, height, cx.zoomed);
    let table_h = (body_rows + 1) as u16;
    let (sort, desc) = g.sort();
    let devices = cx.store.labels(gpu::INFO.id.name).count().max(1);
    let table = if g.derived().rows.is_empty() {
        let replaying = cx.store.sources().any(|s| s.id == gridwatch_store::JOURNAL);
        let text = if cx.store.record(&gpu::PROCS.idx(DEV)).is_some() {
            "no GPU processes"
        } else if replaying {
            // A journal recorded with `--tables off` has no rows to wait for.
            "no process rows in this journal (--tables off)"
        } else {
            "waiting for the process rows…"
        };
        View::Stack {
            dir: Dir::V,
            children: vec![
                (
                    Constraint::Len(1),
                    View::Text(vec![vec![
                        Span::bold(Role::TextMuted, "    PID"),
                        ghost(" "),
                        Span::bold(Role::TextMuted, "TYPE    "),
                        ghost(" "),
                        Span::bold(Role::TextMuted, " GPU"),
                        ghost(" "),
                        Span::bold(Role::TextMuted, "       GPU MEM"),
                        ghost(" "),
                        Span::bold(Role::TextMuted, "Command"),
                    ]]),
                ),
                (Constraint::Fill(1), View::Text(vec![vec![ghost(text)]])),
            ],
        }
    } else {
        let mut enabled: Vec<super::table::Col> = g.columns().to_vec();
        // The zoomed `full` tier shows nvtop's set with USER (§8.1).
        if cx.tier > TIER_PROCS && !enabled.contains(&super::table::Col::User) {
            enabled.insert(1, super::table::Col::User);
        }
        // `h`/`l` scroll the columns four at a time (nvtop's own step,
        // arc 8a). PID and Command always stay: a table of numbers with
        // nothing to name them is not a table anyone can read.
        let scroll = g.col_scroll();
        if scroll > 0 && enabled.len() > 2 {
            let keep_first = enabled.remove(0);
            let command = enabled
                .iter()
                .position(|c| *c == super::table::Col::Command)
                .map(|i| enabled.remove(i));
            let drop = scroll.min(enabled.len());
            enabled.drain(..drop);
            enabled.insert(0, keep_first);
            if let Some(c) = command {
                enabled.push(c);
            }
        }
        super::table::view(
            g.derived(),
            cx.inner.width,
            body_rows,
            g.options().command_min,
            sort,
            desc,
            g.selected(),
            g.scroll(),
            &enabled,
            devices,
        )
    };
    let above = RenderCx {
        inner: Rect {
            height: HEADER_ROWS + band,
            ..cx.inner
        },
        tier: TIER_CHARTS,
        ..*cx
    };
    let mut children = vec![
        (Constraint::Len(HEADER_ROWS + band), charts(g, &above)),
        (Constraint::Len(table_h), table),
    ];
    if cx.tier > TIER_PROCS && g.options().power_panel {
        children.push((
            Constraint::Fill(1),
            View::Text(vec![vec![
                muted("Power "),
                ghost(
                    "board power above; the six 12V-2x6 pins arrive with the pins source (arc 3)",
                ),
            ]]),
        ));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The `F9` picker, drawn over the body it is asking about (arc 8a).
fn signal_menu(g: &Gpu, at: usize) -> View {
    let who = g
        .selected()
        .map(|pid| format!("pid {pid}"))
        .unwrap_or_else(|| "no process".into());
    let mut lines: Vec<Vec<Span>> = vec![
        vec![Span::bold(
            Role::AccentPrimary,
            format!("send a signal to {who}"),
        )],
        vec![Span::new(
            Role::TextGhost,
            "↑/↓ move · Enter apply · Esc cancel",
        )],
        Vec::new(),
    ];
    for (i, (name, _)) in crate::gpu::SIGNALS.iter().enumerate() {
        let cursor = i == at;
        lines.push(vec![
            Span::new(
                if cursor {
                    Role::AccentPrimary
                } else {
                    Role::TextGhost
                },
                if cursor { "▸ " } else { "  " },
            ),
            Span::new(
                if cursor { Role::Text } else { Role::TextMuted },
                (*name).to_string(),
            ),
        ]);
    }
    View::Text(lines)
}

pub fn render(g: &Gpu, cx: &RenderCx<'_>) -> View {
    if let Some(at) = g.signal_menu() {
        return signal_menu(g, at);
    }
    match cx.tier {
        TIER_BADGE => badge(cx),
        TIER_GAUGES => gauges(cx),
        TIER_HEADER => header(g, cx),
        TIER_CHARTS => charts(g, cx),
        _ => table(g, cx),
    }
}

/// Exposed for tests: the effective-load formula (nvtop `extract_gpuinfo.c`).
pub fn effective_load(util: f64, power: f64, limit: f64) -> f64 {
    if limit <= 0.0 {
        return 0.0;
    }
    (util * power / limit).min(100.0)
}
