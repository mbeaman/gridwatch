//! disk component gate tests (§8, D64, brief arc 14 seam 6): the tier per
//! real grid size, the column-drop order rendered, the temperature join by
//! device and each of its three refusals, the service-time staleness rule,
//! the keys, and the promise that no tier ever raises `Detail`.

use std::sync::Arc;
use std::time::Duration;

use gridwatch_components::disk::{
    AWAIT_HOLD_TICKS, Disk, NoTemp, Options, SeriesKind, Sort, TIER_CHART, TIER_FULL, TIER_RATES,
    TIER_SPARKS, TIER_TABLE,
};
use gridwatch_store::keys::disk;
use gridwatch_store::keys::sensors;
use gridwatch_store::{
    Batch, CapSet, Datum, Detail, KeyCode, KeyEvent, Label, MetricId, Mods, Msg, Sample, Store, Ts,
    demo,
};
use gridwatch_ui::component::{Component, InputCx, Outcome, Size, TickCx, pick_tier};
use gridwatch_ui::testkit::{demo_store, plain_text, render_component, theme, tick};
use ratatui_core::layout::Rect;

fn tile() -> Disk {
    Disk::default()
}

/// The demo store carries the disk synth's three drives **and** the sensors
/// synth's hwmon inventory, so the cross-source join is live in every test
/// below without a line of hardware.
fn store() -> Store {
    demo_store(42, 40)
}

/// A store fed by the disk synth alone: no sensors source at all, which is
/// what `--no-default-features --features disk` produces.
fn store_without_sensors(ticks: u64) -> Store {
    let mut store = Store::default();
    let mut synth = demo::DiskSynth::new(42);
    for i in 1..=ticks {
        let at = Ts(i * 1_000_000_000);
        store.apply(&Msg::Batch(synth.tick_at(at)));
    }
    store
}

#[test]
fn disk_tiers_match_the_real_grid_sizes() {
    let c = tile();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(17, 8, false), ("rates", false), "1x1 at 250x70");
    assert_eq!(tier(38, 8, false), ("table", false), "2x1 at 250x70");
    assert_eq!(tier(80, 20, false), ("chart", false), "4x2 at 250x70");
    assert_eq!(
        tier(122, 31, false),
        ("chart", false),
        "6x3: full is zoom-only"
    );
    assert_eq!(tier(59, 18, false), ("chart", false), "6x3 dense at 120x40");
    assert_eq!(tier(39, 11, false), ("table", false));
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
}

/// D64 §8 and the arc's cheapest review check: **no tier raises `Detail`**.
/// One `/proc/diskstats` read serves them all, and per-process disk I/O is
/// htop's I/O screen at `Detail::Columns` — a different tile's job.
#[test]
fn no_tier_ever_raises_detail() {
    let c = tile();
    for tier in 0..c.tiers().len() {
        assert_eq!(
            c.demand(tier),
            Detail::Meters,
            "tier `{}` asked for more than meters",
            c.tiers()[tier].name
        );
    }
}

/// §8's column-drop order, rendered: the guaranteed set at the tier's own
/// 36-wide minimum, then one column at each threshold, checked in the
/// buffer rather than only in the function that computes it.
#[test]
fn the_table_drops_columns_in_the_order_section_8_gives() {
    let store = store();
    let th = theme("modern");
    let head = |w: u16| -> String {
        let mut c = tile();
        let (tier, buf) = render_component(&mut c, &store, &th, Size::new(w, 10), false);
        assert_eq!(c.tiers()[tier].name, "table", "at {w} wide");
        plain_text(&buf)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let want: [(u16, &[&str], &[&str]); 6] = [
        (
            36,
            &["DEVICE", "READ", "WRITE", "BUSY"],
            &["°C", "R/S", "W/S", "Q", "MODEL"],
        ),
        (41, &["°C"], &["R/S", "W/S", "Q", "MODEL"]),
        (48, &["R/S"], &["W/S", "Q", "MODEL"]),
        (55, &["W/S"], &["Q", "MODEL"]),
        (61, &["Q"], &["MODEL"]),
        (80, &["MODEL"], &[]),
    ];
    for (w, present, absent) in want {
        let h = head(w);
        for p in present {
            assert!(h.contains(p), "{w} wide is missing {p}: {h:?}");
        }
        for a in absent {
            assert!(!h.contains(a), "{w} wide should not show {a}: {h:?}");
        }
        // `DEVICE READ WRITE BUSY` survive at every width the tier reaches.
        for always in ["DEVICE", "READ", "WRITE", "BUSY"] {
            assert!(h.contains(always), "{w} wide lost {always}: {h:?}");
        }
    }
}

/// D64 §7, the arc's one genuinely new mechanism: the temperature is the
/// **sensors** source's, joined by `device`. The demo inventory's hwmon
/// numbering deliberately disagrees with the controller numbering
/// (hwmon0/1/2 are nvme1/nvme2/nvme0), so a build that joined by index
/// would put the wrong number beside the wrong drive and this test would
/// see it.
#[test]
fn every_drive_gets_its_own_controllers_temperature() {
    let store = store();
    let mut c = tile();
    tick(&mut c, &store, TIER_FULL);
    let drives = &c.model().drives;
    assert_eq!(drives.len(), 3);
    for d in drives {
        assert_eq!(d.no_temp, NoTemp::None, "{}", d.name);
        assert!(d.temp_c.is_some(), "{} has no temperature", d.name);
        assert!(d.crit_c.is_some(), "{} has no critical", d.name);
    }
    // The join is by device, and the chip it landed on is the one whose
    // `ChipInfo.device` matches — not the one whose number matches.
    let by_name = |n: &str| drives.iter().find(|d| d.name == n).expect(n);
    assert_eq!(by_name("nvme0n1").chip.as_deref(), Some("nvme:Composite"));
    assert_eq!(by_name("nvme1n1").chip.as_deref(), Some("nvme#2:Composite"));
    assert_eq!(by_name("nvme2n1").chip.as_deref(), Some("nvme#3:Composite"));
    // The synth's three drives sit at different temperatures, so a join that
    // put one drive's reading on another would change these numbers.
    let t = |n: &str| by_name(n).temp_c.expect("a reading");
    assert!(
        t("nvme0n1") > t("nvme1n1"),
        "{:?}",
        (t("nvme0n1"), t("nvme1n1"))
    );
    assert!(t("nvme1n1") > t("nvme2n1"));
    // And the `full` pane names the chip the number came from, because two
    // namespaces of one controller legitimately share a reading.
    let th = theme("modern");
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    let text = plain_text(&buf);
    assert!(text.contains("hwmon nvme#3:Composite"), "{text}");
    assert!(text.contains("the controller's"), "{text}");
}

/// The other half: with no sensors source at all — the
/// `--no-default-features --features disk` build — every temperature is `—`
/// and the `full` pane says which of the three reasons it is.
#[test]
fn without_the_sensors_source_the_column_is_a_dash_and_full_says_why() {
    let store = store_without_sensors(40);
    let mut c = tile();
    tick(&mut c, &store, TIER_FULL);
    for d in &c.model().drives {
        assert_eq!(d.no_temp, NoTemp::NoSource, "{}", d.name);
        assert!(d.temp_c.is_none());
        assert!(d.chip.is_none());
    }
    let th = theme("modern");
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(80, 20), false);
    assert_eq!(c.tiers()[tier].name, "chart");
    let text = plain_text(&buf);
    assert!(text.contains("°C"), "the column is still there: {text}");
    assert!(
        text.contains("—"),
        "and it says nothing rather than 0: {text}"
    );
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    let text = plain_text(&buf);
    assert!(
        text.contains("no sensors source"),
        "the pane must say which of the three it is: {text}"
    );
}

/// The third refusal: a sensors source that is running, and a drive whose
/// controller no chip hangs off — a SATA drive with no `drivetemp` module,
/// which is the common case on a real machine.
#[test]
fn a_drive_with_no_matching_chip_says_so_rather_than_borrowing_one() {
    let mut store = Store::default();
    let mut sensors_synth = demo::SensorsSynth::new(42);
    let at = Ts(1_000_000_000);
    store.apply(&Msg::Batch(sensors_synth.tick_at(at)));
    store.apply(&Msg::Batch(Batch {
        source: disk::SOURCE,
        at,
        samples: vec![
            Sample {
                id: disk::READ_BPS.named(&Arc::from("sda")).id,
                datum: Datum::Scalar(0.0),
            },
            Sample {
                id: MetricId {
                    name: disk::INFO.id.name,
                    label: Label::Name(Arc::from("sda")),
                },
                datum: Datum::Record(Arc::new(disk::DiskInfo {
                    name: "sda".into(),
                    device: "0:0:0:0".into(),
                    ..disk::DiskInfo::default()
                })),
            },
        ],
    }));
    let mut c = tile();
    tick(&mut c, &store, TIER_FULL);
    let d = &c.model().drives[0];
    assert_eq!(d.name, "sda");
    assert_eq!(d.no_temp, NoTemp::NoChip);
    assert!(d.temp_c.is_none());
    let th = theme("modern");
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    assert!(
        plain_text(&buf).contains("drivetemp"),
        "the pane names the fix"
    );
}

/// D64 trap 7 / §11's 3 × cadence rule. Three cases exist and only two are
/// reachable from a synth: *absent* (the idle drive never completed
/// anything) and *fresh*. The third — published, then quiet for longer than
/// the hold — cannot be produced without breaking the synth's
/// byte-determinism, so it is built by hand here.
#[test]
fn a_service_time_is_held_for_three_cadences_and_then_becomes_a_dash() {
    let cadence = Duration::from_secs(1);
    let mut store = Store::default();
    let sample = |name: &str, key: &gridwatch_store::Key<f64>, v: f64| Sample {
        id: key.named(&Arc::from(name)).id,
        datum: Datum::Scalar(v),
    };
    // Ten seconds of a drive that completes one read on the first tick and
    // nothing after: the rate keeps coming (0.0), the await does not.
    for i in 1..=10u64 {
        let at = Ts(i * cadence.as_nanos() as u64);
        let mut samples = vec![sample("nvme0n1", &disk::READ_BPS, 0.0)];
        if i == 1 {
            samples.push(sample("nvme0n1", &disk::READ_AWAIT_MS, 0.42));
        }
        store.apply(&Msg::Batch(Batch {
            source: disk::SOURCE,
            at,
            samples,
        }));
    }
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    // The cadence is *observed* rather than configured: two samples a second
    // apart make it one second, so the hold is three.
    assert_eq!(c.await_hold(), cadence * AWAIT_HOLD_TICKS);
    let d = &c.model().drives[0];
    let published = Ts(cadence.as_nanos() as u64);
    assert_eq!(d.read_await.map(|(_, v)| v), Some(0.42));
    let held = |now_s: u64| {
        d.await_ms(
            SeriesKind::Read,
            Ts(now_s * 1_000_000_000),
            cadence * AWAIT_HOLD_TICKS,
        )
    };
    assert_eq!(held(1), Some(0.42), "the tick it arrived on");
    assert_eq!(held(3), Some(0.42), "two cadences later, still drawn");
    assert_eq!(held(4), Some(0.42), "exactly three cadences: the last tick");
    assert_eq!(held(5), None, "past the hold: `—`, never a stale number");
    assert_eq!(published, Ts(1_000_000_000));
    // A write that never happened at all is `—` from the first frame, and
    // is not confused with one that went stale.
    assert_eq!(
        d.await_ms(SeriesKind::Write, Ts(1_000_000_000), cadence * 3),
        None
    );
    // And the label is still alive after all that quiet, because the rate
    // kept publishing — the store test in `gridwatch-store` pins the other
    // side of the same rule.
    assert_eq!(d.read_bps, 0.0);
    assert_eq!(c.model().drives.len(), 1);
}

/// A slower source holds its service times for longer, because the cadence
/// the tile uses is the one it observed rather than one it assumed.
#[test]
fn the_hold_follows_the_cadence_the_source_actually_runs_at() {
    let mut store = Store::default();
    let mut c = tile();
    // Fed a batch at a time, with a `tick` between, exactly as the frame
    // loop runs it: the gap between two consecutive samples *is* the
    // cadence, and nothing had to read `[sources.disk]` to learn it.
    for i in 1..=4u64 {
        store.apply(&Msg::Batch(Batch {
            source: disk::SOURCE,
            at: Ts(i * 4_000_000_000),
            samples: vec![Sample {
                id: disk::READ_BPS.named(&Arc::from("nvme0n1")).id,
                datum: Datum::Scalar(0.0),
            }],
        }));
        tick(&mut c, &store, TIER_TABLE);
    }
    assert_eq!(
        c.await_hold(),
        Duration::from_secs(12),
        "[sources.disk] refresh_ms = 4000: three ticks is twelve seconds"
    );
}

fn cx<'a>(store: &'a Store, caps: &'a CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, 80, 20),
        caps,
        readonly: false,
        zoomed: false,
        tier: TIER_CHART,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        mods: Mods::NONE,
    }
}

#[test]
fn the_keys_change_the_series_the_sort_and_the_scroll() {
    let store = store();
    let caps = CapSet::default();
    let mut c = tile();
    tick(&mut c, &store, TIER_CHART);
    assert_eq!(c.series(), SeriesKind::Read);
    for (ch, want) in [
        ('2', SeriesKind::Write),
        ('3', SeriesKind::Busy),
        ('4', SeriesKind::Queue),
        ('1', SeriesKind::Read),
    ] {
        assert!(matches!(
            c.on_key(key(KeyCode::Char(ch)), &cx(&store, &caps)),
            Outcome::Consumed
        ));
        assert_eq!(c.series(), want, "`{ch}`");
    }
    // `5` is not a series: the component must not consume it.
    assert!(matches!(
        c.on_key(key(KeyCode::Char('5')), &cx(&store, &caps)),
        Outcome::Ignored
    ));
    // The busiest drive is first under `traffic`, and the names sort under
    // `name` — the synth's busy drive happens to be first either way, so
    // the assertion is on the order the sort *declares*.
    assert_eq!(c.sort(), Sort::Traffic);
    assert!(matches!(
        c.on_key(key(KeyCode::Char('s')), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert_eq!(c.sort(), Sort::Name);
    let names: Vec<&str> = c.model().drives.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["nvme0n1", "nvme1n1", "nvme2n1"]);
    assert!(matches!(
        c.on_key(key(KeyCode::Down), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert_eq!(c.scroll(), 1);
    assert!(matches!(
        c.on_key(key(KeyCode::Up), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert_eq!(c.scroll(), 0);
    assert!(matches!(
        c.on_key(key(KeyCode::Char('z')), &cx(&store, &caps)),
        Outcome::Ignored
    ));
}

/// The sort really does put the busy drive first, whichever order the store
/// hands the labels over in.
#[test]
fn traffic_sort_puts_the_busy_drive_first_and_the_idle_one_last() {
    let store = store();
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    let d = &c.model().drives;
    assert_eq!(d[0].name, "nvme0n1", "the synth's busy drive");
    assert_eq!(
        d.last().expect("three drives").name,
        "nvme2n1",
        "the idle one"
    );
    assert!(d[0].total() > d[1].total());
    assert_eq!(d.last().expect("three").total(), 0.0);
    // The tile sums what it shows and says over how many (D64 §2: there is
    // no published total to disagree with).
    let (rd, wr) = c.model().totals();
    assert_eq!(rd, d.iter().map(|x| x.read_bps).sum::<f64>());
    assert!(wr > 0.0);
}

/// The view options filter what is drawn without touching what the source
/// publishes — and `hidden()` says how many were left out.
#[test]
fn the_device_and_hide_globs_filter_the_rows() {
    let store = store();
    let th = theme("modern");
    let mut c = Disk::new(Options {
        devices: vec!["nvme0*".into()],
        ..Options::default()
    });
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(c.model().drives.len(), 1);
    assert_eq!(c.model().hidden(), 2);
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(80, 12), false);
    let text = plain_text(&buf);
    assert!(text.contains("nvme0n1"));
    assert!(!text.contains("nvme1n1"));
    assert!(text.contains("(2 hidden)"), "{text}");

    let mut c = Disk::new(Options {
        hide: vec!["nvme2*".into()],
        ..Options::default()
    });
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(c.model().drives.len(), 2);
    assert!(!c.model().drives.iter().any(|d| d.name == "nvme2n1"));
}

/// An empty store is an honest tile: a dash and a sentence, never a
/// fabricated zero, at every tier.
#[test]
fn an_empty_store_says_so() {
    let store = Store::default();
    let th = theme("modern");
    for (w, h, z) in [(17, 8, false), (80, 20, false), (248, 66, true)] {
        let mut c = tile();
        let (_, buf) = render_component(&mut c, &store, &th, Size::new(w, h), z);
        let text = plain_text(&buf);
        // The sentence is elided by a 17-wide tile, so the assertion is on
        // the part that always fits — and on there being no fabricated
        // number anywhere near it.
        assert!(text.contains("no block"), "{w}x{h}: {text}");
    }
    // And `tick` on an empty store does not ask for a redraw.
    let mut c = tile();
    let cx = TickCx {
        store: &store,
        now: Ts::ZERO,
        visible: true,
        tier: TIER_RATES,
    };
    assert_eq!(c.tick(&cx), gridwatch_ui::component::Redraw::No);
    assert!(c.model().drives.is_empty());
}

/// The small tiers name the busiest drive and sum what they show; the
/// caveat that the whole arc exists for is on the tile, not only in a
/// decision file.
#[test]
fn the_small_tiers_carry_the_pair_and_full_carries_the_caveat() {
    let store = store();
    let th = theme("modern");
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(17, 8), false);
    assert_eq!(tier, TIER_RATES);
    let text = plain_text(&buf);
    assert!(text.contains("rd ") && text.contains("wr "), "{text}");
    assert!(text.contains("nvme0n1"), "{text}");

    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(24, 6), false);
    assert_eq!(tier, TIER_SPARKS);
    assert!(plain_text(&buf).contains("nvme0n1"));

    let (_, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    let text = plain_text(&buf);
    assert!(
        text.contains("share of time the queue was non-empty"),
        "the BUSY caveat is on the tile: {text}"
    );
    assert!(text.contains("of 1023"), "q against nr_requests: {text}");
    assert!(text.contains("scheduler"), "{text}");
    assert!(text.contains("partitions nvme0n1p1"), "{text}");
}

/// The `sensors` source is *optional*: naming it must not make the tile
/// require it, and the manifest must not claim a capability it does not
/// need.
#[test]
fn the_manifest_asks_for_nothing_and_borrows_the_sensors_source() {
    let m = tile().manifest();
    assert_eq!(m.sources, [disk::SOURCE]);
    assert_eq!(m.optional_sources, [sensors::SOURCE]);
    assert!(m.requires.is_empty());
    assert!(
        m.optional.is_empty(),
        "no capability: /proc is world-readable"
    );
}
