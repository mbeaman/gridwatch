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
use gridwatch_ui::component::{Component, InputCx, Outcome, Size, pick_tier};
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

/// A chip that stops answering must not stay on the tile for ever: the
/// store has no retraction, so the tile drops readings older than a few
/// cadences (review).
#[test]
fn a_reading_that_stopped_arriving_leaves_the_tile() {
    let store = store_with(&[("nvme:Composite", 80.0, Some(81.85), None)]);
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(c.model().temps.len(), 1);
    let mut m = gridwatch_components::sensors::Model::default();
    m.refresh(&store, &[], Sort::Hottest, Ts(1_000_000_000));
    assert_eq!(m.temps.len(), 1, "fresh");
    m.refresh(&store, &[], Sort::Hottest, Ts(20_000_000_000));
    assert!(m.temps.is_empty(), "19 s old: gone");
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
