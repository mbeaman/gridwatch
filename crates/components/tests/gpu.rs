//! gpu component gate tests (§8, §8.1, brief 2b): the tier the real grid hands
//! each rect, the row budget, gridwatch's drop order, the join with its
//! last-known cache, nvtop's ENC/DEC hiding, the chart series and the keys.

use std::sync::Arc;

use gridwatch_components::gpu::table::{Col, fit_columns};
use gridwatch_components::gpu::{Gpu, OPTION_NAMES, Options, TIER_CHARTS, TIER_PROCS};
use gridwatch_store::keys::cpu::ProcTable;
use gridwatch_store::keys::gpu::{GpuProcKind, GpuProcRow, GpuProcs};
use gridwatch_store::keys::{cpu, gpu};
use gridwatch_store::{Batch, Datum, Detail, KeyCode, KeyEvent, Mods, Msg, Sample, Store, Ts};
use gridwatch_ui::component::{Component, InputCx, Outcome, Size, pick_tier};
use gridwatch_ui::testkit::{demo_store, demo_store_at, plain_text, render_component, theme};
use ratatui_core::layout::Rect;

fn gpu() -> Gpu {
    Gpu::default()
}

/// The tier the real grid hands each rect (§6 measured sizes).
#[test]
fn gpu_tiers_match_the_real_grid_sizes() {
    let c = gpu();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(122, 31, false), ("procs", false), "6x3 at 250x70");
    assert_eq!(tier(59, 18, false), ("procs", false), "6x3 dense at 120x40");
    assert_eq!(tier(80, 20, false), ("procs", false), "4x2 at 250x70");
    assert_eq!(tier(56, 17, false), ("charts", false));
    assert_eq!(tier(56, 11, false), ("header", false));
    assert_eq!(tier(38, 8, false), ("gauges", false), "2x1 at 250x70");
    assert_eq!(tier(17, 8, false), ("badge", false), "1x1 at 250x70");
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
    assert_eq!(tier(248, 66, false), ("procs", false), "full is zoom-only");
}

/// §8.1 row budget: 10 at 250×70 (14 available under an 8-row band), 7 in a
/// 4x2 (4-row band), 5 at the dense floor and 6 one row taller; zoom fills.
#[test]
fn row_budget_at_the_real_grid_sizes() {
    let g = gpu();
    assert_eq!(g.band_rows(TIER_PROCS, 31), 8);
    assert_eq!(g.body_rows(TIER_PROCS, 31, false), 10);
    assert_eq!(g.band_rows(TIER_PROCS, 20), 4);
    assert_eq!(g.body_rows(TIER_PROCS, 20, false), 7);
    assert_eq!(g.body_rows(TIER_PROCS, 18, false), 5);
    assert_eq!(g.body_rows(TIER_PROCS, 19, false), 6);
    assert_eq!(g.body_rows(TIER_PROCS, 66, true), 66 - 8 - 8 - 1);
    // The charts tier's band grows with height up to eight rows.
    assert_eq!(g.band_rows(TIER_CHARTS, 12), 4);
    assert_eq!(g.band_rows(TIER_CHARTS, 17), 8);
}

/// Gridwatch's drop order (§8.1): ENC, DEC, HOST MEM, TYPE, USER, CPU; PID,
/// GPU, GPU MEM and Command always survive; DEV only with several devices.
#[test]
fn columns_drop_in_gridwatch_order() {
    let all: Vec<Col> = [
        "pid", "user", "dev", "type", "gpu", "enc", "dec", "gpu_mem", "cpu", "host_mem", "command",
    ]
    .iter()
    .map(|c| Col::from_id(c).unwrap())
    .collect();
    let ids = |cols: &[Col]| cols.iter().map(|c| c.id()).collect::<Vec<_>>();
    // Wide: everything but DEV (one device).
    let wide = fit_columns(&all, 200, 12, 8, 1);
    assert_eq!(
        ids(&wide),
        [
            "pid", "user", "type", "gpu", "enc", "dec", "gpu_mem", "cpu", "host_mem", "command"
        ]
    );
    assert!(fit_columns(&all, 200, 12, 8, 2).contains(&Col::Dev));
    // The tier's minimum width with the grid default: HOST MEM goes, Command keeps 12.
    let default: Vec<Col> = gridwatch_components::gpu::DEFAULT_COLUMNS
        .iter()
        .map(|c| Col::from_id(c).unwrap())
        .collect();
    let narrow = fit_columns(&default, 56, 12, 4, 1);
    assert_eq!(
        ids(&narrow),
        ["pid", "type", "gpu", "gpu_mem", "cpu", "command"],
        "56 wide"
    );
    // Absurdly narrow: the four survivors.
    let tiny = fit_columns(&all, 20, 12, 8, 1);
    assert_eq!(ids(&tiny), ["pid", "gpu", "gpu_mem", "command"]);
}

fn procs_store(rows: Vec<GpuProcRow>, table: Option<ProcTable>, at: u64) -> Store {
    let mut store = Store::default();
    store.apply(&Msg::Batch(Batch {
        source: gpu::SOURCE,
        at: Ts(at),
        samples: vec![Sample {
            id: gpu::PROCS.idx(0).id,
            datum: Datum::Record(Arc::new(GpuProcs {
                rows,
                vram_total_b: 32607 << 20,
            })),
        }],
    }));
    if let Some(t) = table {
        store.apply(&Msg::Batch(Batch {
            source: cpu::SOURCE,
            at: Ts(at),
            samples: vec![Sample {
                id: cpu::PROC_TABLE.id.clone(),
                datum: Datum::Record(Arc::new(t)),
            }],
        }));
    }
    store
}

fn row(pid: i32, kind: GpuProcKind, mib: u64, sm: u32) -> GpuProcRow {
    GpuProcRow {
        pid,
        kind,
        vram_b: Some(mib << 20),
        sm_pct: sm,
        mem_pct: 0,
        enc_pct: 0,
        dec_pct: 0,
        fresh: sm > 0,
    }
}

fn tick(g: &mut Gpu, store: &Store) {
    gridwatch_ui::testkit::tick(g, store, TIER_PROCS);
}

/// The join (§8.1): USER/CPU/HOST MEM/Command from `proc.table`, `—` without
/// it, the last-known cmdline kept when the scan no longer lists the PID, and
/// the tie rule (sm 0 below active at equal memory).
#[test]
fn join_uses_the_scan_and_keeps_the_last_known_command() {
    let table = gridwatch_store::demo::proc_table(0, 42);
    let rows = vec![
        row(412345, GpuProcKind::Both, 12800, 17),
        row(1701, GpuProcKind::Graphics, 464, 3),
        row(11805, GpuProcKind::Compute, 44, 0),
        row(999_999, GpuProcKind::Graphics, 44, 0), // unknown to the scan
    ];
    let mut g = gpu();
    let store = procs_store(rows.clone(), Some(table), 1_000);
    tick(&mut g, &store);
    let pids: Vec<i32> = g.rows().iter().map(|r| r.pid).collect();
    // Default sort gpu_mem desc; the two 44 MiB rows tie → sm 0 both → PID.
    assert_eq!(pids, vec![412345, 1701, 11805, 999_999]);
    let game = &g.rows()[0];
    assert_eq!(game.user.as_deref(), Some("mattbeam"));
    assert!(
        game.cmdline
            .as_deref()
            .unwrap()
            .contains("/opt/game/bin/game")
    );
    assert!(game.cpu_pct.is_some() && game.res_kib.is_some());
    let unknown = &g.rows()[3];
    assert!(unknown.cmdline.is_none() && unknown.user.is_none());

    // The scan stops listing the game (a new table without it): cmdline stays.
    let mut table2 = gridwatch_store::demo::proc_table(1, 42);
    table2.rows.retain(|r| r.pid != 412345);
    let store = procs_store(rows, Some(table2), 2_000);
    tick(&mut g, &store);
    let game = g.rows().iter().find(|r| r.pid == 412345).unwrap();
    assert!(game.cmdline.is_some(), "last-known cmdline kept");
    assert!(game.cpu_pct.is_none(), "but no live CPU% for it");

    // No cpu source at all: every joined column is absent.
    let mut g = gpu();
    let store = procs_store(vec![row(1701, GpuProcKind::Graphics, 464, 3)], None, 3_000);
    tick(&mut g, &store);
    let th = theme("mono");
    let (_, buf) = render_component(&mut g, &store, &th, Size::new(122, 31), false);
    let text = plain_text(&buf);
    assert!(
        text.contains("[1701]"),
        "no cmdline ever read → [pid]:\n{text}"
    );
    assert!(text.contains("—"), "the joined columns say —");
}

/// The keys (arc 2's read-only set): select, page, sort cycle, invert, and
/// the chart toggles.
#[test]
fn keys_select_sort_invert_and_toggle_series() {
    let store = demo_store(42, 6);
    let mut g = gpu();
    tick(&mut g, &store);
    assert!(g.rows().len() >= 4, "the synth's gpu set");
    let caps = gridwatch_store::CapSet::empty();
    let cx = InputCx {
        store: &store,
        inner: Rect::new(0, 0, 122, 31),
        caps: &caps,
        readonly: false,
    };
    let key = |c: KeyCode| KeyEvent {
        code: c,
        mods: Mods::NONE,
    };
    assert!(g.selected().is_none());
    g.on_key(key(KeyCode::Down), &cx);
    assert_eq!(g.selected(), Some(g.rows()[0].pid));
    g.on_key(key(KeyCode::End), &cx);
    assert_eq!(g.selected(), Some(g.rows().last().unwrap().pid));
    let (col, desc) = g.sort();
    assert_eq!((col, desc), (Col::GpuMem, true));
    g.on_key(key(KeyCode::Char('>')), &cx);
    assert_eq!(g.sort().0, Col::Cpu, "next enabled column after gpu_mem");
    g.on_key(key(KeyCode::Char('I')), &cx);
    assert!(!g.sort().1, "inverted");
    // Selection follows the PID across the re-sort.
    let pid = g.selected().unwrap();
    assert!(g.rows().iter().any(|r| r.pid == pid));
    assert_eq!(g.series_on(), &[true, true, false, true, false, false]);
    g.on_key(key(KeyCode::Char('3')), &cx);
    assert!(g.series_on()[2], "temp toggled on");
    assert!(!g.reversed());
    g.on_key(key(KeyCode::Char('r')), &cx);
    assert!(g.reversed());
}

/// nvtop's ENC/DEC bars hide 30 s after the last non-zero reading.
#[test]
fn enc_dec_bars_hide_after_thirty_idle_seconds() {
    let mut store = Store::default();
    let feed = |store: &mut Store, at: u64, enc: f64| {
        store.apply(&Msg::Batch(Batch {
            source: gpu::SOURCE,
            at: Ts(at),
            samples: vec![
                Sample {
                    id: gpu::ENC_PCT.idx(0).id,
                    datum: Datum::Scalar(enc),
                },
                Sample {
                    id: gpu::DEC_PCT.idx(0).id,
                    datum: Datum::Scalar(0.0),
                },
                Sample {
                    id: gpu::UTIL_PCT.idx(0).id,
                    datum: Datum::Scalar(10.0),
                },
            ],
        }));
    };
    let mut g = gpu();
    feed(&mut store, 1_000_000_000, 0.0);
    tick(&mut g, &store);
    assert!(g.encdec_visible_at(Ts(1_000_000_000)), "shown at first");
    feed(&mut store, 31_000_000_000, 0.0);
    tick(&mut g, &store);
    assert!(
        !g.encdec_visible_at(Ts(31_000_000_000)),
        "hidden after 30 s idle"
    );
    feed(&mut store, 32_000_000_000, 40.0);
    tick(&mut g, &store);
    assert!(
        g.encdec_visible_at(Ts(32_000_000_000)),
        "activity brings them back"
    );
    let th = theme("mono");
    let (_, buf) = render_component(&mut g, &store, &th, Size::new(56, 8), false);
    assert!(
        plain_text(&buf).contains("ENC"),
        "the bar is drawn while active"
    );
}

/// The header's PCIe half survives the tier's minimum width, and the MEM bar
/// is VRAM occupancy, not memory-controller load (digest §1).
#[test]
fn header_keeps_pcie_at_minimum_width_and_mem_is_vram() {
    let store = demo_store(42, 6);
    let th = theme("mono");
    let mut g = gpu();
    let (tier, buf) = render_component(&mut g, &store, &th, Size::new(56, 8), false);
    assert_eq!(g.tiers()[tier].name, "header");
    let text = plain_text(&buf);
    assert!(text.contains("PCIe GEN 5@16x"), "{text}");
    assert!(text.contains("RX:") && text.contains("TX:"));
    assert!(text.contains("POW"));
    // 13.9/32.6 GiB ≈ 43 %, far from the synth's memctl (≈ 5 %).
    let mem_line = text.lines().find(|l| l.contains("MEM")).unwrap();
    assert!(
        mem_line.contains("4") && !mem_line.contains(" 5%"),
        "MEM bar is VRAM occupancy: {mem_line}"
    );
}

/// The chart band's series: util, vram, power on by default; the zoomed
/// `full` tier adds USER and the Power placeholder.
#[test]
fn charts_and_full_tier_content() {
    let store = demo_store(42, 40);
    let th = theme("mono");
    let mut g = gpu();
    let (tier, buf) = render_component(&mut g, &store, &th, Size::new(122, 31), false);
    assert_eq!(g.tiers()[tier].name, "procs");
    let text = plain_text(&buf);
    assert!(text.contains("1:util") && text.contains("2:vram") && text.contains("4:power"));
    // A minute of synth history: the window label follows the run's age,
    // capped at nvtop's ten minutes.
    assert!(text.contains("1m ⟶"), "the window label: {text}");
    assert!(
        text.contains("SMs") && text.contains("170"),
        "the spec column at 122 wide"
    );
    assert!(text.contains("Both G+C"), "the game's merged TYPE");
    assert!(
        text.contains("12.5GiB") || text.contains("12.5G"),
        "host memory joined"
    );
    let (tier, buf) = render_component(&mut g, &store, &th, Size::new(248, 66), true);
    assert_eq!(g.tiers()[tier].name, "full");
    let text = plain_text(&buf);
    assert!(text.contains("USER") && text.contains("mattbeam"));
    assert!(text.contains("Power"), "the Power sub-panel placeholder");
    // Meters-only store: the table tier says it is waiting, no fabricated rows.
    let meters = demo_store_at(42, 6, Detail::Meters);
    let mut g = gpu();
    let (_, buf) = render_component(&mut g, &meters, &th, Size::new(122, 31), false);
    let text = plain_text(&buf);
    assert!(text.contains("waiting for the process rows"), "{text}");
}

/// The `clock` series divides by a ceiling published once per generation;
/// it must have as many points as `util` (review: it was empty).
#[test]
fn clock_series_has_points_despite_its_static_ceiling() {
    let store = demo_store(42, 40);
    let th = theme("mono");
    let mut g = gpu();
    let caps = gridwatch_store::CapSet::empty();
    let cx = InputCx {
        store: &store,
        inner: Rect::new(0, 0, 122, 31),
        caps: &caps,
        readonly: false,
    };
    g.on_key(
        KeyEvent {
            code: KeyCode::Char('5'),
            mods: Mods::NONE,
        },
        &cx,
    );
    let view = gridwatch_ui::testkit::view_of(&mut g, &store, &th, Size::new(122, 31));
    let dump = gridwatch_ui::dump::view_value(&view).to_string();
    // The chart node lists every series with its point count.
    let v: serde_json::Value = serde_json::from_str(&dump).unwrap();
    let mut counts = std::collections::BTreeMap::new();
    fn walk(v: &serde_json::Value, out: &mut std::collections::BTreeMap<String, u64>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(serde_json::Value::Array(series)) = m.get("chart") {
                    for s in series {
                        out.insert(
                            s["label"].as_str().unwrap_or("").to_string(),
                            s["points"].as_u64().unwrap_or(0),
                        );
                    }
                }
                m.values().for_each(|x| walk(x, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(&v, &mut counts);
    assert!(counts["util"] > 10, "{counts:?}");
    assert_eq!(counts["clock"], counts["util"], "{counts:?}");
}

#[test]
fn option_names_match_the_options_struct() {
    let table = toml::Table::try_from(Options::default()).expect("options serialise");
    let fields: Vec<&str> = table.keys().map(String::as_str).collect();
    let mut listed = OPTION_NAMES.to_vec();
    listed.sort_unstable();
    assert_eq!(fields, listed, "OPTION_NAMES has drifted from Options");
}

#[test]
fn options_are_validated_through_build() {
    let bad: toml::Table = toml::from_str("sort = \"nonsense\"").unwrap();
    assert!(Gpu::from_table(&bad).is_err());
    let bad: toml::Table = toml::from_str("series = [\"heat\"]").unwrap();
    assert!(Gpu::from_table(&bad).is_err());
    let ok: toml::Table = toml::from_str("table_rows = 2\ncolumns = [\"pid\", \"enc\"]").unwrap();
    let g = Gpu::from_table(&ok).unwrap();
    assert_eq!(g.options().table_rows, 5, "nvtop/htop's five-row floor");
}

/// Arc 8a: nvtop's horizontal scroll and its `F9` signal menu, in the
/// zoom-only `full` tier only. No process is touched: the action is read
/// as data.
#[test]
fn the_full_tier_scrolls_columns_and_offers_the_signal_menu() {
    use gridwatch_ui::component::Command;
    let store = demo_store(42, 6);
    let th = theme("modern");
    let caps = gridwatch_store::CapSet::default();
    let mut g = Gpu::new(Options::default());
    tick(&mut g, &store);
    let zoom = Size::new(248, 66);
    let cx = |inner: Rect| InputCx {
        store: &store,
        inner,
        caps: &caps,
        readonly: false,
    };
    let big = Rect::new(0, 0, zoom.w, zoom.h);
    let small = Rect::new(0, 0, 80, 20);

    // On the grid the keys do nothing: a 4x2 tile has nowhere to scroll.
    assert!(matches!(
        g.on_key(KeyEvent::plain(KeyCode::Char('l')), &cx(small)),
        Outcome::Ignored
    ));
    assert_eq!(g.col_scroll(), 0);
    assert!(matches!(
        g.on_key(KeyEvent::plain(KeyCode::F(9)), &cx(small)),
        Outcome::Ignored
    ));

    // Zoomed, `l` scrolls four columns and `h` comes back.
    g.on_key(KeyEvent::plain(KeyCode::Char('l')), &cx(big));
    assert_eq!(g.col_scroll(), 4);
    let (_, buf) = render_component(&mut g, &store, &th, zoom, true);
    let text = plain_text(&buf);
    assert!(text.contains("PID"), "PID always survives a scroll: {text}");
    assert!(text.contains("Command"), "and so does Command: {text}");
    g.on_key(KeyEvent::plain(KeyCode::Char('h')), &cx(big));
    assert_eq!(g.col_scroll(), 0);

    // `F9` needs a selected row, then offers htop's signal list.
    g.on_key(KeyEvent::plain(KeyCode::Down), &cx(big));
    assert!(g.selected().is_some());
    g.on_key(KeyEvent::plain(KeyCode::F(9)), &cx(big));
    assert_eq!(g.signal_menu(), Some(0));
    let (_, buf) = render_component(&mut g, &store, &th, zoom, true);
    let text = plain_text(&buf);
    assert!(text.contains("send a signal to"), "{text}");
    assert!(text.contains("SIGTERM") && text.contains("SIGKILL"));

    // Enter hands the shell an action naming that pid, and asks first.
    let out = g.on_key(KeyEvent::plain(KeyCode::Enter), &cx(big));
    let Outcome::Command(Command::Run(_, action)) = out else {
        panic!("no action from the gpu signal menu");
    };
    assert!(format!("{action:?}").contains("SIGTERM"));
    assert_eq!(action.pids().len(), 1);
    assert!(action.confirm().is_some());
    assert!(g.signal_menu().is_none());

    // Esc closes it without building anything.
    g.on_key(KeyEvent::plain(KeyCode::F(9)), &cx(big));
    assert!(matches!(
        g.on_key(KeyEvent::plain(KeyCode::Esc), &cx(big)),
        Outcome::Consumed
    ));
    assert!(g.signal_menu().is_none());
}
