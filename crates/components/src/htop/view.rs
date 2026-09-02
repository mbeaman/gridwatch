//! The view builders: one function per tier, each laying itself out from the
//! real inner rect (§4.6 — the rect and the tier index are the whole truth).
//! Nothing here reads a file, names a colour or picks a glyph; every width is
//! derived with saturating arithmetic so the no-panic sweep from 0×0 upwards
//! can never index past a row.

use std::borrow::Cow;
use std::time::Duration;

use gridwatch_store::keys::cpu::{CoreBreakdown, CpuTopology};
use gridwatch_store::keys::{cpu, sys};
use std::sync::Arc;

use gridwatch_store::{Agg, Key, Store};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{Constraint, Dir, Span, View};
use ratatui_core::layout::Rect;

use super::format as fmt;
use super::{Htop, ROWS_ABOVE_TABLE, TIER_BIG_NUMBER, TIER_CORES, TIER_METERS, TIER_TINY};

/// How much history the sparkline shows at most; one bucket per cell. A run
/// younger than this spans only what it has lived, so the line starts drawing
/// on the second sample instead of after three minutes.
const SPARK_SPAN: Duration = Duration::from_secs(180);

/// Two columns of meters once the tile is at least this wide — htop's
/// `two_50_50` header layout.
const TWO_COLUMN_W: u16 = 76;

/// One blank column between the two meter columns, so a bar's text can never
/// touch the next label.
const GUTTER: u16 = 2;

fn scalar(store: &Store, key: &Key<f64>) -> Option<f64> {
    store.last(key).map(|(_, v)| v)
}

/// `sensor.temp_c{chip:label}` — the cpu source owns k10temp until arc 5 (§8).
fn temp_c(store: &Store, label: &str) -> Option<f64> {
    let key = cpu::TEMP_C.named(&Arc::from(label));
    store.last(&key).map(|(_, v)| v)
}

fn breakdown(store: &Store, core: Option<u16>) -> Option<CoreBreakdown> {
    let key = match core {
        None => cpu::BREAKDOWN,
        Some(n) => cpu::BREAKDOWN.idx(n),
    };
    store.record(&key).map(|(_, b)| *b)
}

fn topology(store: &Store) -> Option<CpuTopology> {
    store.record(&cpu::TOPOLOGY).map(|(_, t)| t.clone())
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

/// htop's CPU meter segments, mapped to roles by meaning, never by colour:
/// nice → tertiary accent (htop's blue), user → ok (green), kernel → crit
/// (red), virt → info (cyan). iowait is **not** drawn: htop counts it as idle
/// unless `detailed_cpu_time` is on.
fn cpu_segments(b: CoreBreakdown) -> Vec<(Role, f32)> {
    vec![
        (Role::AccentTertiary, b.nice.max(0.0)),
        (Role::Ok, b.user.max(0.0)),
        (Role::Crit, b.kernel.max(0.0)),
        (Role::Info, b.virt.max(0.0)),
    ]
}

fn cpu_meter(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let total = scalar(store, &cpu::TOTAL_PCT);
    let segments = breakdown(store, None)
        .map(cpu_segments)
        .unwrap_or_else(|| vec![(Role::Ok, 0.0)]);
    View::Segmented {
        label: "CPU".into(),
        segments,
        text: Some(fmt::pct(total).into()),
    }
}

/// htop's memory meter: used / shared / buffers / cache, in that order, as
/// fractions of MemTotal. The segments sum to `total − free` by construction
/// (`used` already excludes cache, buffers and SReclaimable), so the bar can
/// never overflow. `compressed` (zswap) is out — torch has zswap disabled.
fn mem_meter(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let total = scalar(store, &cpu::MEM_TOTAL_B).unwrap_or(0.0);
    let frac = |v: Option<f64>| {
        if total > 0.0 {
            (v.unwrap_or(0.0) / total) as f32
        } else {
            0.0
        }
    };
    let used = scalar(store, &cpu::MEM_USED_B);
    let shared = scalar(store, &cpu::MEM_SHARED_B);
    let text = if total > 0.0 {
        format!(
            "{}/{}",
            fmt::human_bytes(used.unwrap_or(0.0) + shared.unwrap_or(0.0)),
            fmt::human_bytes(total)
        )
    } else {
        "—".into()
    };
    View::Segmented {
        label: "MEM".into(),
        segments: vec![
            (Role::Ok, frac(used)),
            (Role::AccentPrimary, frac(shared)),
            (
                Role::AccentTertiary,
                frac(scalar(store, &cpu::MEM_BUFFERS_B)),
            ),
            (Role::Warn, frac(scalar(store, &cpu::MEM_CACHED_B))),
        ],
        text: Some(text.into()),
    }
}

/// htop's swap meter: used / cache. `frontswap` (zswap) is out with the memory
/// meter's `compressed` segment.
fn swap_meter(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let total = scalar(store, &cpu::SWAP_TOTAL_B).unwrap_or(0.0);
    let used = scalar(store, &cpu::SWAP_USED_B);
    let cached = scalar(store, &cpu::SWAP_CACHED_B);
    let frac = |v: Option<f64>| {
        if total > 0.0 {
            (v.unwrap_or(0.0) / total) as f32
        } else {
            0.0
        }
    };
    let text = if total > 0.0 {
        format!(
            "{}/{}",
            fmt::human_bytes(used.unwrap_or(0.0)),
            fmt::human_bytes(total)
        )
    } else {
        "none".into()
    };
    View::Segmented {
        label: "SWP".into(),
        segments: vec![(Role::Crit, frac(used)), (Role::Warn, frac(cached))],
        text: Some(text.into()),
    }
}

fn spark(cx: &RenderCx<'_>, buckets: u16) -> View {
    let mut out = Vec::new();
    // `cx.now` is time since the run's epoch (§4.1), so this is the run's age.
    let span = Duration::from_nanos(cx.now.0)
        .min(SPARK_SPAN)
        .max(Duration::from_secs(1));
    cx.store.resample(
        &cpu::TOTAL_PCT,
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

/// htop's wording once the scan has counted kernel threads; the honest
/// pids/tasks form before it (PARITY.md); shortened clause by clause to fit.
fn tasks_text(cx: &RenderCx<'_>, width: u16) -> String {
    let store = cx.store;
    fmt::tasks_fit(
        scalar(store, &sys::TASKS_TOTAL),
        scalar(store, &sys::TASKS_THREADS),
        scalar(store, &sys::TASKS_KERNEL),
        scalar(store, &sys::TASKS_RUNNING),
        usize::from(width),
    )
}

/// `load 4.51 4.20 3.52 · up 3d 06:01`
fn load_line(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let mut spans = vec![
        muted("load "),
        value(fmt::load(
            scalar(store, &sys::LOAD1),
            scalar(store, &sys::LOAD5),
            scalar(store, &sys::LOAD15),
        )),
    ];
    if let Some(up) = scalar(store, &sys::UPTIME_S) {
        spans.push(ghost(" · "));
        spans.push(muted("up "));
        spans.push(value(fmt::uptime(up)));
    }
    View::Text(vec![spans])
}

/// `636 tasks, 2338 thr; 3 running · load 0.66 0.52 0.53 · up 1d 23:34`,
/// trimmed from the right as the tile narrows.
fn info_line(cx: &RenderCx<'_>, width: u16) -> View {
    let store = cx.store;
    let tasks = tasks_text(cx, width);
    let load = fmt::load(
        scalar(store, &sys::LOAD1),
        scalar(store, &sys::LOAD5),
        scalar(store, &sys::LOAD15),
    );
    let up = scalar(store, &sys::UPTIME_S)
        .map(fmt::uptime)
        .unwrap_or_else(|| "—".into());
    let mut spans = vec![value(tasks)];
    // Each clause is added only if the whole clause fits: half a load average
    // is worse than none.
    let mut used: u16 = spans
        .iter()
        .map(|s| s.text.chars().count() as u16)
        .sum::<u16>();
    for (sep, label, body) in [(" · ", "load ", load), (" · ", "up ", up)] {
        // Columns, not bytes: " · " is four bytes and three cells.
        let cost = (sep.chars().count() + label.chars().count() + body.chars().count()) as u16;
        if used.saturating_add(cost) > width {
            break;
        }
        used += cost;
        spans.push(ghost(sep));
        spans.push(muted(label));
        spans.push(value(body));
    }
    View::Text(vec![spans])
}

/// `PSI  cpu 0.42 · mem 0.00 · io 0.03` — the `some avg10` triple. torch has no
/// `pressure/irq` (CONFIG_IRQ_TIME_ACCOUNTING is off), so there is no IRQ row.
fn psi_line(cx: &RenderCx<'_>) -> View {
    let store = cx.store;
    let mut spans = vec![muted("PSI ")];
    for (i, (name, key)) in [
        ("cpu", &cpu::PSI_CPU),
        ("mem", &cpu::PSI_MEM),
        ("io", &cpu::PSI_IO),
    ]
    .into_iter()
    .enumerate()
    {
        if i > 0 {
            spans.push(ghost(" · "));
        }
        spans.push(muted(format!("{name} ")));
        match scalar(store, key) {
            Some(v) => spans.push(Span::new(
                if v >= 10.0 { Role::Warn } else { Role::Text },
                format!("{v:.2}"),
            )),
            None => spans.push(ghost("—")),
        }
    }
    View::Text(vec![spans])
}

/// Bar geometry for one CCD block. A physical core is two SMT bars `tw` cells
/// wide with `inner` between them; `outer` separates cores, so a pair reads as
/// a pair. The widest geometry that fits wins; the floor is htop's own three
/// cells per core, which is what makes the 56-wide tier minimum work
/// (8 cores × 3 cells × 2 blocks + labels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geom {
    tw: u16,
    inner: u16,
    outer: u16,
}

impl Geom {
    fn core_w(self) -> u16 {
        self.tw * 2 + self.inner
    }

    fn width(self, cores: u16) -> u16 {
        cores.saturating_mul(self.core_w()) + cores.saturating_sub(1) * self.outer
    }
}

fn geom(block_w: u16, cores: u16) -> Geom {
    const FLOOR: Geom = Geom {
        tw: 1,
        inner: 0,
        outer: 1,
    };
    if cores == 0 {
        return FLOOR;
    }
    for tw in (1..=4u16).rev() {
        let g = Geom {
            tw,
            inner: 1,
            outer: 2,
        };
        if g.width(cores) <= block_w {
            return g;
        }
    }
    FLOOR
}

/// One CCD block: header (id · mean MHz · Tccd), the paired SMT bars, and a
/// label row of core ids aligned under each pair.
fn ccd_block(
    cx: &RenderCx<'_>,
    die: u16,
    cores: &[Vec<u16>],
    topo: &CpuTopology,
    width: u16,
) -> View {
    let store = cx.store;
    let g = geom(width, cores.len() as u16);
    // Before the second scan there are no per-core percentages at all; a chart
    // of zeroes would read as "every core idle" rather than "no delta yet".
    let have_data = cores
        .iter()
        .flatten()
        .any(|c| store.last(&cpu::CORE_PCT.idx(*c)).is_some());

    // Bars: two per physical core (a zero-height column is the gap).
    let mut values: Vec<f32> = Vec::with_capacity(g.width(cores.len() as u16) as usize);
    for (i, pair) in cores.iter().enumerate() {
        if i > 0 {
            values.extend(std::iter::repeat_n(0.0, g.outer as usize));
        }
        for t in 0..2usize {
            if t > 0 {
                values.extend(std::iter::repeat_n(0.0, g.inner as usize));
            }
            // A core with no SMT sibling still occupies its slot.
            let pct = pair
                .get(t)
                .and_then(|c| scalar(store, &cpu::CORE_PCT.idx(*c)))
                .unwrap_or(0.0);
            values.extend(std::iter::repeat_n(
                (pct / 100.0).clamp(0.0, 1.0) as f32,
                g.tw as usize,
            ));
        }
    }

    // Header: mean frequency over the block, and the die's own Tccd.
    let freqs: Vec<f64> = cores
        .iter()
        .flatten()
        .filter_map(|c| scalar(store, &cpu::FREQ_MHZ.idx(*c)))
        .collect();
    let mean_mhz = if freqs.is_empty() {
        f64::NAN
    } else {
        freqs.iter().sum::<f64>() / freqs.len() as f64
    };
    let temp = topo.temp_label(die).and_then(|l| temp_c(store, l));
    let short = width < 26;
    let mut header = vec![Span::bold(Role::AccentSecondary, format!("CCD{die}"))];
    header.push(ghost("  "));
    header.push(muted(fmt::ghz(mean_mhz, short)));
    if let Some(t) = temp {
        header.push(ghost("  "));
        header.push(Span::new(
            if t >= 85.0 {
                Role::Crit
            } else {
                Role::TextMuted
            },
            fmt::celsius(t, short),
        ));
    }

    // Labels: the physical core id, centred under its SMT pair.
    let mut labels = String::new();
    for (i, pair) in cores.iter().enumerate() {
        if i > 0 {
            labels.push_str(&" ".repeat(g.outer as usize));
        }
        let id = topo
            .core_of
            .get(pair.first().copied().unwrap_or(0) as usize)
            .copied()
            .unwrap_or(i as u16);
        let span = g.core_w() as usize;
        let text = format!("{id}");
        let text = if text.len() > span {
            text[text.len() - span..].to_string()
        } else {
            text
        };
        let pad = span.saturating_sub(text.len());
        let left = pad / 2;
        labels.push_str(&" ".repeat(left));
        labels.push_str(&text);
        labels.push_str(&" ".repeat(pad - left));
    }

    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![header])),
            (
                Constraint::Fill(1),
                if have_data {
                    View::Bars {
                        values,
                        gradient: GradientId::Load,
                        labels: None,
                        peaks: None,
                    }
                } else {
                    View::Text(vec![vec![ghost("waiting for the first delta…")]])
                },
            ),
            (Constraint::Len(1), View::Text(vec![vec![muted(labels)]])),
        ],
    }
}

/// The CCD blocks, side by side: one column per die, which is what makes the
/// 56-wide minimum work (8 cores × 3 cells × 2 blocks + labels) and keeps the
/// bars tall — the tile's height is the bar chart's resolution.
fn core_blocks(cx: &RenderCx<'_>, width: u16) -> View {
    let Some(topo) = topology(cx.store) else {
        return View::Text(vec![vec![ghost("waiting for the cpu topology…")]]);
    };
    let dies = topo.dies();
    if dies.is_empty() {
        return View::Empty;
    }
    let n = dies.len() as u16;
    let each_w = width / n.max(1);
    let children = dies
        .iter()
        .map(|(die, cores)| {
            (
                Constraint::Fill(1),
                ccd_block(cx, *die, cores, &topo, each_w),
            )
        })
        .collect();
    View::Stack {
        dir: Dir::H,
        children,
    }
}

/// What a header panel swallowed, so the caller does not print it twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Panel {
    rows: u16,
    info_inside: bool,
    psi_inside: bool,
}

/// htop's `two_50_50` header: the CPU meter and its history on the left, memory
/// and the system lines on the right. Fills exactly `rows`.
fn two_column_panel(cx: &RenderCx<'_>, width: u16, rows: u16) -> (View, Panel) {
    let half = width.saturating_sub(GUTTER) / 2;
    let left = View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), cpu_meter(cx)),
            (Constraint::Fill(1), spark(cx, half)),
        ],
    };
    let info_inside = rows >= 4;
    let psi_inside = rows >= 5;
    let mut right = vec![
        (Constraint::Len(1), mem_meter(cx)),
        (Constraint::Len(1), swap_meter(cx)),
    ];
    if info_inside {
        right.push((
            Constraint::Len(1),
            View::Text(vec![vec![value(tasks_text(cx, half))]]),
        ));
        right.push((Constraint::Len(1), load_line(cx)));
    }
    if psi_inside {
        right.push((Constraint::Len(1), psi_line(cx)));
    }
    right.push((Constraint::Fill(1), View::Empty));
    let block = View::Stack {
        dir: Dir::H,
        children: vec![
            (
                Constraint::Fill(1),
                if rows > 1 { left } else { cpu_meter(cx) },
            ),
            (Constraint::Len(GUTTER), View::Empty),
            (
                Constraint::Fill(1),
                View::Stack {
                    dir: Dir::V,
                    children: right,
                },
            ),
        ],
    };
    (
        block,
        Panel {
            rows,
            info_inside,
            psi_inside,
        },
    )
}

/// The narrow header: three stacked meters, nothing else.
fn one_column_panel(cx: &RenderCx<'_>) -> (View, Panel) {
    (
        View::Stack {
            dir: Dir::V,
            children: vec![
                (Constraint::Len(1), cpu_meter(cx)),
                (Constraint::Len(1), mem_meter(cx)),
                (Constraint::Len(1), swap_meter(cx)),
            ],
        },
        Panel {
            rows: 3,
            info_inside: false,
            psi_inside: false,
        },
    )
}

/// `tiny` [3]: the total percentage and a sparkline of it.
fn tiny(cx: &RenderCx<'_>) -> View {
    let w = cx.inner.width;
    let total = scalar(cx.store, &cpu::TOTAL_PCT);
    let head = if w >= 11 {
        vec![
            muted("CPU "),
            Span::bold(Role::AccentSecondary, fmt::pct(total)),
        ]
    } else if w >= 8 {
        vec![
            muted("CPU "),
            Span::bold(Role::AccentSecondary, fmt::pct_short(total)),
        ]
    } else {
        vec![Span::bold(Role::AccentSecondary, fmt::pct_short(total))]
    };
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![head])),
            (Constraint::Fill(1), spark(cx, w)),
        ],
    }
}

/// `big-number` [4]: htop's number as big digits, with the sparkline under it
/// whenever the rect has rows to spare.
fn big_number(cx: &RenderCx<'_>) -> View {
    let Some(total) = scalar(cx.store, &cpu::TOTAL_PCT) else {
        // The big-text font has no glyph for the `—` sentinel and draws
        // *nothing* for a character it cannot render, so a tile with no delta
        // yet would be silently blank. Fall back to the tier below, which says
        // `CPU —` in plain text.
        return tiny(cx);
    };
    let total = Some(total);
    // Quadrant digits are four cells wide: keep the '%' only when it fits.
    let text = if cx.inner.width >= 16 {
        fmt::pct_short(total)
    } else {
        fmt::pct_short(total).trim_end_matches('%').to_string()
    };
    let big = View::BigNumber {
        text: text.into(),
        role: Role::AccentSecondary,
    };
    if cx.inner.height <= 4 {
        return big;
    }
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(4), big),
            (Constraint::Fill(1), spark(cx, cx.inner.width)),
        ],
    }
}

/// `meters` [6]: htop's header — the CPU/MEM/SWP meters plus tasks, load and
/// uptime — with the sparkline taking whatever rows are left.
fn meters(cx: &RenderCx<'_>) -> View {
    let w = cx.inner.width;
    let h = cx.inner.height;
    if w >= TWO_COLUMN_W {
        // The panel owns the whole tile; its sparkline takes the spare rows.
        let (panel, p) = two_column_panel(cx, w, h);
        let mut children = vec![(Constraint::Fill(1), panel)];
        if !p.info_inside {
            children.push((Constraint::Len(1), info_line(cx, w)));
        }
        if !p.psi_inside && h >= 4 {
            children.push((Constraint::Len(1), psi_line(cx)));
        }
        return View::Stack {
            dir: Dir::V,
            children,
        };
    }
    let (panel, p) = one_column_panel(cx);
    let mut children = vec![
        (Constraint::Len(p.rows), panel),
        (Constraint::Len(1), info_line(cx, w)),
    ];
    let used = p.rows + 1;
    // The sparkline is `tiny`'s own content, so it outranks the pressure row
    // when only one line fits: a richer tier never drops a poorer tier's datum.
    if h > used + 1 {
        children.push((Constraint::Fill(1), spark(cx, w)));
    }
    if h > used + 2 {
        children.push((Constraint::Len(1), psi_line(cx)));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// `cores` [12]: the meters, then one block per CCD with its cores as paired
/// SMT bars, then the pressure row.
fn cores(cx: &RenderCx<'_>) -> View {
    let w = cx.inner.width;
    let h = cx.inner.height;
    // The header takes a quarter of a tall tile (its sparkline grows with it);
    // everything else is the bar chart, whose height is its resolution.
    let (panel, p) = if w >= TWO_COLUMN_W {
        two_column_panel(cx, w, (h / 4).clamp(2, 8))
    } else {
        one_column_panel(cx)
    };
    let mut children = vec![(Constraint::Len(p.rows), panel)];
    if !p.info_inside {
        children.push((Constraint::Len(1), info_line(cx, w)));
    }
    children.push((Constraint::Fill(1), core_blocks(cx, w)));
    if !p.psi_inside {
        children.push((Constraint::Len(1), psi_line(cx)));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// `table` [12 + 1 + N]: `cores` in its 12 rows (more when the table is
/// capped), then the top-N process table of §8.1 — `rows = min(table_rows,
/// available)` on the grid, everything when zoomed.
fn table(h: &Htop, cx: &RenderCx<'_>) -> View {
    let height = cx.inner.height;
    let available = usize::from(height.saturating_sub(ROWS_ABOVE_TABLE + 1));
    let body_rows = if cx.zoomed {
        available
    } else {
        available.min(usize::from(h.options().table_rows))
    };
    let table_h = (body_rows + 1) as u16;
    let cores_cx = RenderCx {
        inner: Rect {
            height: height.saturating_sub(table_h),
            ..cx.inner
        },
        tier: TIER_CORES,
        ..*cx
    };
    let (sort, desc) = h.sort();
    let table = if h.derived().rows.is_empty() {
        let text = if cx.store.record(&cpu::PROC_TABLE).is_some() {
            "no processes to show (every row is filtered out)"
        } else {
            "waiting for the process scan…"
        };
        View::Stack {
            dir: Dir::V,
            children: vec![
                (
                    Constraint::Len(1),
                    View::Text(vec![vec![
                        Span::bold(Role::TextMuted, "PID"),
                        ghost("  "),
                        Span::bold(Role::TextMuted, "CPU%"),
                        ghost("  "),
                        Span::bold(Role::TextMuted, "Command"),
                    ]]),
                ),
                (Constraint::Fill(1), View::Text(vec![vec![ghost(text)]])),
            ],
        }
    } else {
        super::table::view(
            h.derived(),
            h.options(),
            cx.inner.width,
            body_rows,
            sort,
            desc,
            h.selected(),
            h.scroll(),
            h.columns(),
        )
    };
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Fill(1), cores(&cores_cx)),
            (Constraint::Len(table_h), table),
        ],
    }
}

pub fn render(h: &Htop, cx: &RenderCx<'_>) -> View {
    match cx.tier {
        TIER_TINY => tiny(cx),
        TIER_BIG_NUMBER => big_number(cx),
        TIER_METERS => meters(cx),
        TIER_CORES => cores(cx),
        _ => table(h, cx),
    }
}
