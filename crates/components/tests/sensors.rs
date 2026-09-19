//! sensors component gate tests (§8, brief arc 5 seam 7): the tier per real
//! grid size, the hottest/over-max/over-crit roles, the chip filter and the
//! sort key, the gpu row with and without the gpu source, the RAPL footer,
//! and the honest empty tile.

use std::sync::Arc;

use gridwatch_components::sensors::{Options, Sensors, Sort, TIER_TABLE};
use gridwatch_store::keys::sensors;
use gridwatch_store::{
    Batch, Datum, KeyCode, KeyEvent, Label, MetricId, Mods, Msg, Sample, Store, Ts,
};
use gridwatch_ui::component::{Component, InputCx, Outcome, Size, TickCx, pick_tier};
use gridwatch_ui::testkit::{demo_store, plain_text, render_component, theme, tick};
use ratatui_core::layout::Rect;

fn tile() -> Sensors {
    Sensors::default()
}

#[test]
fn sensors_tiers_match_the_real_grid_sizes() {
    let c = tile();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(17, 8, false), ("hottest", false), "1x1 at 250x70");
    assert_eq!(tier(38, 8, false), ("strip", false), "2x1 at 250x70");
    assert_eq!(tier(80, 20, false), ("chart", false), "4x2 at 250x70");
    assert_eq!(
        tier(122, 31, false),
        ("chart", false),
        "6x3: full is zoom-only"
    );
    assert_eq!(tier(40, 8, false), ("table", false));
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
    // The default 6x1 slot on the Overview.
    assert_eq!(tier(122, 8, false), ("table", false), "6x1 at 250x70");
}

/// A store with one chip over its max and one under.
fn store_with(temps: &[(&str, f64, Option<f64>, Option<f64>)]) -> Store {
    let mut store = Store::default();
    let mut samples = Vec::new();
    for (key, v, max, crit) in temps {
        let id = |k: &gridwatch_store::Key<f64>| MetricId {
            name: k.id.name,
            label: Label::Name(Arc::from(*key)),
        };
        samples.push(Sample {
            id: id(&sensors::TEMP_C),
            datum: Datum::Scalar(*v),
        });
        if let Some(m) = max {
            samples.push(Sample {
                id: id(&sensors::MAX_C),
                datum: Datum::Scalar(*m),
            });
        }
        if let Some(c) = crit {
            samples.push(Sample {
                id: id(&sensors::CRIT_C),
                datum: Datum::Scalar(*c),
            });
        }
    }
    samples.push(Sample {
        id: sensors::INFO.id.clone(),
        datum: Datum::Record(Arc::new(sensors::SensorsInfo {
            chips: Vec::new(),
            rapl: sensors::RaplState::RootOnly,
        })),
    });
    store.apply(&Msg::Batch(Batch {
        source: sensors::SOURCE,
        at: Ts(1_000_000_000),
        samples,
    }));
    store
}

#[test]
fn the_hottest_is_the_closest_to_its_limit_and_roles_follow_max_and_crit() {
    let store = store_with(&[
        ("k10temp:Tctl", 59.0, None, None),
        ("nvme:Composite", 80.0, Some(81.85), Some(84.85)),
        ("spd5118:temp1", 44.0, Some(55.0), Some(85.0)),
    ]);
    let th = theme("modern");
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(17, 8), false);
    assert_eq!(c.tiers()[tier].name, "hottest");
    let text = plain_text(&buf);
    // nvme is at 98 % of its max; the DIMM at 80 %; k10temp exports no max
    // and is ranked against the assumed 95 °C (62 %) — it is *shown*, which
    // ranking by the raw margin never did (review).
    assert!(text.contains("nvme"), "{text}");
    // 17 cells: `nvme Composite 80` — the degree sign is the first thing
    // the width takes.
    assert!(text.contains("80"), "{text}");
    let hottest = c.model().hottest().unwrap();
    assert_eq!(hottest.key, "nvme:Composite");
    assert!(!hottest.over_max() && !hottest.over_crit());
    let keys: Vec<&str> = c.model().temps.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["nvme:Composite", "spd5118:temp1", "k10temp:Tctl"]);
    let tctl = c
        .model()
        .temps
        .iter()
        .find(|r| r.chip == "k10temp")
        .unwrap();
    assert!(tctl.assumed());
    assert_eq!(tctl.limit(), 95.0, "AMD documents Tctl's ceiling");
    assert!((tctl.heat() - 59.0 / 95.0).abs() < 1e-9);
    // A reading past its crit outranks everything, however cool it is.
    let store = store_with(&[
        ("nvme:Composite", 60.0, Some(81.85), Some(84.85)),
        ("spd5118:temp1", 86.0, Some(55.0), Some(85.0)),
    ]);
    let mut c2 = tile();
    tick(&mut c2, &store, TIER_TABLE);
    assert_eq!(c2.model().hottest().unwrap().chip, "spd5118");
    // Over max, then over crit.
    let store = store_with(&[("nvme:Composite", 86.0, Some(81.85), Some(84.85))]);
    let mut c = tile();
    // 24 wide: the chip, the label, the reading and the over-max mark fit.
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(24, 8), false);
    let text = plain_text(&buf);
    assert!(text.contains("▲"), "over max marks the reading: {text}");
    let r = c.model().hottest().unwrap();
    assert!(r.over_max() && r.over_crit());
}

fn cx<'a>(store: &'a Store, caps: &'a gridwatch_store::CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, 80, 20),
        caps,
        readonly: false,
        zoomed: false,
        tier: 0,
    }
}

#[test]
fn the_sort_key_and_the_chip_filter() {
    let store = store_with(&[
        ("k10temp:Tctl", 59.0, None, None),
        ("nvme:Composite", 80.0, Some(81.85), None),
        ("spd5118:temp1", 44.0, Some(55.0), None),
    ]);
    let caps = gridwatch_store::CapSet::default();
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    let keys: Vec<&str> = c.model().temps.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys[0], "nvme:Composite",
        "hottest = the closest to its limit"
    );
    assert_eq!(c.sort(), Sort::Hottest);
    assert!(matches!(
        c.on_key(
            KeyEvent {
                code: KeyCode::Char('o'),
                mods: Mods::NONE
            },
            &cx(&store, &caps)
        ),
        Outcome::Consumed
    ));
    assert_eq!(c.sort(), Sort::Chip);
    let keys: Vec<&str> = c.model().temps.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["k10temp:Tctl", "nvme:Composite", "spd5118:temp1"]);
    // The filter is a view option.
    let mut only = Sensors::new(Options {
        chips: vec!["nvme*".into()],
        sort: Sort::Hottest,
    });
    tick(&mut only, &store, TIER_TABLE);
    assert_eq!(only.model().temps.len(), 1);
    assert_eq!(only.model().temps[0].chip, "nvme");
}

/// The `full` tier's gpu row reads the gpu source's keys; without them it
/// says so. The RAPL line names the udev rule when the source reported
/// `root_only`.
#[test]
fn the_full_tier_carries_rapl_psi_and_the_gpu_row() {
    let th = theme("modern");
    let store = store_with(&[("k10temp:Tctl", 59.0, None, None)]);
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    assert_eq!(c.tiers()[tier].name, "full");
    let text = plain_text(&buf);
    assert!(text.contains("RAPL"), "{text}");
    assert!(text.contains("udev"), "the root-only hint: {text}");
    assert!(text.contains("PSI"), "{text}");
    assert!(text.contains("no gpu source"), "{text}");
    // With every synth (gpu included) the row carries the card's numbers.
    let demo = demo_store(42, 40);
    let mut c = tile();
    let (_, buf) = render_component(&mut c, &demo, &th, Size::new(248, 66), true);
    let text = plain_text(&buf);
    assert!(!text.contains("no gpu source"), "{text}");
    // The fan really reads (the label is `dev:fan`, not the device index).
    let fan = text
        .split("fan ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .unwrap_or("");
    assert!(fan.ends_with('%') && fan.len() > 1, "the gpu fan: {fan:?}");
}

/// One more temperature reading from `source`, at second `at`.
fn push_temp(store: &mut Store, source: gridwatch_store::SourceId, at: u64, key: &str, v: f64) {
    store.apply(&Msg::Batch(Batch {
        source,
        at: Ts(at * 1_000_000_000),
        samples: vec![Sample {
            id: MetricId {
                name: sensors::TEMP_C.id.name,
                label: Label::Name(Arc::from(key)),
            },
            datum: Datum::Scalar(v),
        }],
    }));
}

/// `hwmon0:Composite` — hottest thing on the machine (90 °C) and alphabetically
/// first — is mentioned by the first batch and never again; `nvme:Composite`
/// reports on. Six batches: the silent chip is five periods behind against a
/// hold of three (the rule is *strictly* greater, so exactly three is still
/// live — the first version of this test sat on that boundary). Both sorts
/// would put it first if liveness were ignored, so neither can pass by
/// accident.
fn hwmon0_leaves() -> Store {
    let mut store = store_with(&[
        ("hwmon0:Composite", 90.0, Some(100.0), None),
        ("nvme:Composite", 60.0, Some(100.0), None),
    ]);
    for i in 2..=6u64 {
        push_temp(&mut store, sensors::SOURCE, i, "nvme:Composite", 60.0);
    }
    store
}

/// A chip the source stopped mentioning **keeps its row and says so** (D68
/// §5: "sensors switches from silent drop to the same treatment"). The first
/// pass replaced the 5 s literal with the shared rule and kept the `continue`,
/// so a vanished chip left no trace — the pre-arc behaviour on a different
/// timer — and this test used to *assert* the drop.
#[test]
fn a_chip_the_source_stopped_mentioning_keeps_its_row_and_says_so() {
    let store = hwmon0_leaves();
    let caps = gridwatch_store::CapSet::default();
    let th = theme("modern");
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    let order =
        |c: &Sensors| -> Vec<String> { c.model().temps.iter().map(|r| r.key.clone()).collect() };
    assert_eq!(
        order(&c),
        ["nvme:Composite", "hwmon0:Composite"],
        "hottest: a frozen 90° does not outrank a live 60°, and the row is still here"
    );
    assert!(matches!(
        c.on_key(
            KeyEvent {
                code: KeyCode::Char('o'),
                mods: Mods::NONE
            },
            &cx(&store, &caps)
        ),
        Outcome::Consumed
    ));
    assert_eq!(c.sort(), Sort::Chip);
    assert_eq!(
        order(&c),
        ["nvme:Composite", "hwmon0:Composite"],
        "chip order: alphabetical would put hwmon0 first; a quiet row sinks under every sort"
    );
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(60, 8), false);
    assert_eq!(c.tiers()[tier].name, "table");
    let text = plain_text(&buf);
    let line = text
        .lines()
        .find(|l| l.contains("hwmon0"))
        .unwrap_or_else(|| panic!("the row is drawn — it says the chip was here:\n{text}"));
    assert!(
        line.contains('—') && !line.contains("90"),
        "a chip nobody is reporting draws dashes, not 90°: {line}"
    );
}

/// The summary tiers name the hottest reading. A chip that left carrying the
/// biggest number on the machine must not keep the title.
#[test]
fn a_vanished_chip_is_not_the_hottest() {
    let store = hwmon0_leaves();
    let th = theme("modern");
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(
        c.model().hottest().map(|r| r.key.as_str()),
        Some("nvme:Composite"),
        "the hottest of the readings still arriving"
    );
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(17, 8), false);
    assert_eq!(c.tiers()[tier].name, "hottest");
    let text = plain_text(&buf);
    assert!(text.contains("60"), "the live reading: {text}");
    assert!(!text.contains("90"), "the frozen one: {text}");
}

/// When **every** reading has gone quiet there is no hottest one, and the
/// summary tiers must not name the last frozen leader anyway (the sink in
/// `hottest_first` only helps while a live reading exists to outrank it — the
/// mutation that removed the filter from `hottest()` survived until this test).
#[test]
fn when_every_reading_is_quiet_nothing_is_the_hottest() {
    let mut store = store_with(&[("nvme:Composite", 90.0, Some(100.0), None)]);
    // The source keeps publishing — a fan, not a temperature — so it advances
    // past the one chip that stopped.
    for i in 2..=6u64 {
        store.apply(&Msg::Batch(Batch {
            source: sensors::SOURCE,
            at: Ts(i * 1_000_000_000),
            samples: vec![Sample {
                id: MetricId {
                    name: sensors::FAN_RPM.id.name,
                    label: Label::Name(Arc::from("fan1")),
                },
                datum: Datum::Scalar(1200.0),
            }],
        }));
    }
    let th = theme("modern");
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(c.model().temps.len(), 1, "the row is still there");
    assert!(
        c.model().hottest().is_none(),
        "but it is not the hottest of anything: {:?}",
        c.model().temps
    );
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(17, 8), false);
    assert_eq!(c.tiers()[tier].name, "hottest");
    let text = plain_text(&buf);
    assert!(
        !text.contains("90"),
        "the summary names a frozen reading:\n{text}"
    );
}

/// **The candidate rule.** D68 §3 said the cpu source publishes
/// `sensor.temp_c{k10temp:*}` "by default", so a reading is judged against
/// both sources and is live if either says so. It does not: the default is
/// `k10temp = false` whenever the `sensors` feature is compiled in, which it is
/// by default, so in the shipped build the cpu source carries *no* temperature
/// and a slow cpu source only vetoed. At the default cadences that held a gone
/// chip ~4.5 s too long; at `refresh_ms = 60000` for the whole retention
/// window. Only `k10temp:*` labels have a second candidate.
#[test]
fn the_cpu_source_cannot_hold_a_chip_it_never_carried() {
    // Fed the way the shell feeds it: a tick after every batch, because a
    // `Pulse` learns a source's period from two consecutive observations. (A
    // first draft ticked once at the end, so the cpu pulse still assumed a
    // 1 s period and the veto this test is about could not happen.)
    let mut store = store_with(&[
        ("hwmon0:Composite", 90.0, Some(100.0), None),
        ("nvme:Composite", 60.0, Some(100.0), None),
    ]);
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    for i in 2..=6u64 {
        push_temp(&mut store, sensors::SOURCE, i, "nvme:Composite", 60.0);
        // The cpu source, slowly (every 3 s), publishing no temperatures.
        if i % 3 == 0 {
            store.apply(&Msg::Batch(Batch {
                source: gridwatch_store::keys::cpu::SOURCE,
                at: Ts(i * 1_000_000_000),
                samples: vec![],
            }));
        }
        tick(&mut c, &store, TIER_TABLE);
    }
    assert_eq!(
        c.model().hottest().map(|r| r.key.as_str()),
        Some("nvme:Composite"),
        "hwmon0 is not a k10temp label: only the sensors source could have carried it, \
         it has moved on five periods, and a slower cpu source has no say"
    );
}

/// And the leniency the second candidate exists for: with `k10temp = true` the
/// cpu source carries `k10temp:*`, and judged against the sensors source alone
/// those readings would go quiet whenever the cpu source is slower than three
/// sensors periods. Here cpu publishes every 10 s and sensors every 1 s.
#[test]
fn a_k10temp_reading_the_cpu_carries_survives_the_sensors_clock() {
    let mut store = store_with(&[("nvme:Composite", 40.0, Some(100.0), None)]);
    for i in 2..=28u64 {
        push_temp(&mut store, sensors::SOURCE, i, "nvme:Composite", 40.0);
    }
    for at in [10u64, 20] {
        push_temp(
            &mut store,
            gridwatch_store::keys::cpu::SOURCE,
            at,
            "k10temp:Tctl",
            55.0,
        );
    }
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(
        c.model().hottest().map(|r| r.key.as_str()),
        Some("k10temp:Tctl"),
        "sensors has moved on eight seconds, but the cpu source — which carries this \
         label — spoke at 20 s and is current"
    );
}

/// A source that is merely slow, or has stopped, must not have its readings
/// declared dead. Nothing here advances, so nothing is quiet however much
/// *wall* time passes — proven by ticking with a `now` an hour later. The
/// first version ticked against `store.latest()`, so the pre-arc
/// `now.since(at) > 5 s` drop stayed green (arc 17 review, lens E).
#[test]
fn a_stalled_source_does_not_lose_its_readings() {
    let store = store_with(&[("nvme:Composite", 80.0, Some(81.85), None)]);
    let mut c = tile();
    for now in [store.latest(), Ts(3_600 * 1_000_000_000)] {
        c.tick(&TickCx {
            store: &store,
            now,
            visible: true,
            tier: TIER_TABLE,
        });
        assert_eq!(
            c.model().temps.len(),
            1,
            "an hour of wall time kills nothing"
        );
    }
    assert_eq!(
        c.model().hottest().map(|r| r.key.as_str()),
        Some("nvme:Composite")
    );
}

#[test]
fn an_empty_store_says_so() {
    let th = theme("modern");
    let empty = Store::default();
    for size in [Size::new(17, 8), Size::new(80, 20), Size::new(248, 66)] {
        let mut c = tile();
        let (_, buf) = render_component(&mut c, &empty, &th, size, size.h > 60);
        let text = plain_text(&buf);
        assert!(text.contains("no sensors") || text.contains("—"), "{text}");
        assert!(!text.contains("°C"), "nothing fabricated: {text}");
    }
}

/// D65 §2, the headline of arc 15: the `sensor` column was `Elastic` and sat
/// *before* the fixed ones, so at the reference 250×70 the sensor name was at
/// column 17 and its value at column 104 — eighty-seven blank cells between a
/// row's identity and its number, on every terminal since arc 5b. The
/// renderer's cap (D65 §3) ends the table at its content.
#[test]
fn the_value_sits_beside_the_sensor_it_belongs_to() {
    let store = demo_store(42, 40);
    let th = theme("modern");
    let mut c = tile();
    // The tile's own 6x1 slot on the Overview at 250×70.
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(122, 8), false);
    let text = plain_text(&buf);
    let header = text.lines().next().expect("a header row");
    let sensor = header.find("sensor").expect("the sensor column");
    let value = header.find("value").expect("the value column");
    assert!(
        value - sensor < 20,
        "the value column is {} cells from the sensor column:\n{text}",
        value - sensor
    );
}

/// D65 §7: the warn/crit bars §8 has promised since arc 5b. One `Len(1)`
/// gauge per *shown* row under a header spacer, so each bar sits on the row it
/// pictures — and each row therefore carries its percentage twice, once in the
/// `of lim` column it is sorted by and once at the far end of its own bar.
#[test]
fn the_bars_sit_on_the_rows_they_picture() {
    let store = demo_store(42, 40);
    let th = theme("modern");
    let mut c = tile();
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(122, 8), false);
    let text = plain_text(&buf);
    let rows: Vec<&str> = text
        .lines()
        .skip(1)
        .filter(|l| l.contains("°C"))
        .collect::<Vec<_>>();
    assert!(rows.len() >= 4, "too few reading rows:\n{text}");
    for row in &rows {
        assert_eq!(
            row.matches('%').count(),
            2,
            "a row without its bar's percentage:\n{text}"
        );
    }
    // The header row carries no bar: the spacer is what lines the bars up.
    let header = text.lines().next().expect("a header row");
    assert!(!header.contains('%'), "the spacer row drew a bar: {header}");
}

/// …and below the width the bars need, the table is the whole tier — a bar
/// short enough to read as a chip is worse than no bar (arc 14's `MODEL`
/// rule).
#[test]
fn a_narrow_tile_draws_no_bars() {
    let store = demo_store(42, 40);
    let th = theme("modern");
    let mut c = tile();
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(48, 8), false);
    let text = plain_text(&buf);
    for row in text.lines().filter(|l| l.contains("°C")) {
        assert_eq!(row.matches('%').count(), 1, "a bar at 48 wide:\n{text}");
    }
}

/// The zoom-only `full` tier placed its table at `Len(readings + 1)` and told
/// it its body was `readings`, so `scroll` could never leave zero and the
/// cursor walked off the bottom of any rect shorter than the list. torch's
/// eight readings hide it; forty do not. (§4.6, D65 §5 — the viewport is the
/// band the table was given.)
#[test]
fn the_full_tier_scrolls_a_long_reading_list() {
    let th = theme("modern");
    let readings: Vec<(String, f64, Option<f64>, Option<f64>)> = (0..40)
        // Coolest last: the table is ranked by closeness to a limit, so
        // `chip39` is the row furthest below the fold.
        .map(|i| {
            (
                format!("chip{i:02}:t"),
                79.0 - f64::from(i),
                Some(100.0),
                None,
            )
        })
        .collect();
    let refs: Vec<(&str, f64, Option<f64>, Option<f64>)> = readings
        .iter()
        .map(|(k, v, m, c)| (k.as_str(), *v, *m, *c))
        .collect();
    let store = store_with(&refs);
    let caps = gridwatch_store::CapSet::default();
    let mut c = tile();
    let size = Size::new(100, 24);
    tick(&mut c, &store, 4);
    let (_, buf) = render_component(&mut c, &store, &th, size, true);
    assert!(
        !plain_text(&buf).contains("chip39"),
        "the fixture must not fit"
    );
    let cx = InputCx {
        store: &store,
        inner: Rect {
            x: 0,
            y: 0,
            width: size.w,
            height: size.h,
        },
        caps: &caps,
        readonly: false,
        zoomed: true,
        tier: 4,
    };
    for _ in 0..39 {
        c.on_key(
            KeyEvent {
                code: KeyCode::Down,
                mods: Mods::NONE,
            },
            &cx,
        );
    }
    let (_, buf) = render_component(&mut c, &store, &th, size, true);
    let text = plain_text(&buf);
    assert!(
        text.contains("chip39"),
        "the selected reading is off screen after 39 downs:\n{text}"
    );
}

/// `full` is cumulative over `table` and `chart` (§4.6: a tier draws the one
/// below it plus its own `adds`). It drew neither the warn/crit bars nor the
/// chart when arc 15 added them, so `z` on a wide terminal made the tile
/// strictly *poorer* than the one it zoomed — twenty drawn rows of a hundred
/// and thirty-one (arc 15 review, F1). No snapshot could see it: at every
/// grid size the snapshot matrix picks `chart`, not `full`.
#[test]
fn the_zoomed_tier_keeps_the_bars_and_the_chart() {
    let store = demo_store(7, 40);
    let th = theme("modern");
    let mut t = tile();
    tick(&mut t, &store, 1);
    let (tier, buf) = render_component(&mut t, &store, &th, Size::new(248, 66), true);
    assert_eq!(
        t.tiers()[tier].name,
        "full",
        "a zoomed 248x66 tile must reach the zoom-only tier"
    );
    let text = plain_text(&buf);
    assert!(
        text.chars().any(|c| matches!(c, '━' | '█' | '▓' | '#')),
        "the zoomed tier must draw the warn/crit bars the `table` tier below it draws \
         (the theme picks the glyph):\n{text}"
    );
    assert!(
        text.contains("chart ·"),
        "the zoomed tier must draw the chart the `chart` tier below it draws:\n{text}"
    );
    let drawn = text.lines().filter(|l| !l.trim().is_empty()).count();
    assert!(
        drawn > 40,
        "a 66-row zoomed tile drawing {drawn} rows is the F1 regression again:\n{text}"
    );
}
