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
    // Every nvme device joins — including the partition, which shares its
    // parent's controller. The removable does **not**: no hwmon chip hangs
    // off `0:0:0:0`, which is D64's `NoChip` path and had no fixture at all
    // until arc 16. Asserting "every drive has a temperature" is only
    // possible on a fixture where every drive is the same kind of drive.
    for d in drives {
        if d.name == "sda" {
            assert_eq!(d.no_temp, NoTemp::NoChip, "the removable has no chip");
            assert!(d.temp_c.is_none(), "and therefore no reading, not a zero");
            continue;
        }
        assert_eq!(d.no_temp, NoTemp::None, "{}", d.name);
        assert!(d.temp_c.is_some(), "{} has no temperature", d.name);
        assert!(d.crit_c.is_some(), "{} has no critical", d.name);
    }
    assert!(
        drives.iter().any(|d| d.no_temp == NoTemp::NoChip),
        "a device without a chip must be in the fixture, or the `—` path is untested"
    );
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
    // Sorted by name — asserted as *sortedness* over whatever the fixture
    // holds, not as a literal list, which went stale the first time the
    // fixture gained a device (arc 16; the same defect D66 found twice).
    let names: Vec<&str> = c.model().drives.iter().map(|d| d.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "the name sort is not sorted by name");
    assert!(names.len() > 3, "and the fixture is worth sorting");
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
    // **Among the devices still being reported.** A quiet one sinks below
    // every live row whatever its last numbers were (D68 §5), and at this
    // fixture's 60 s the removable has been gone fifteen seconds — so the
    // idle drive is last of the live, not last of the list.
    let live: Vec<&str> = d
        .iter()
        .filter(|x| !x.live.is_quiet())
        .map(|x| x.name.as_str())
        .collect();
    assert_eq!(
        *live.last().expect("live drives"),
        "nvme2n1",
        "the idle one, of those still reporting"
    );
    assert!(
        d.last().expect("drives").live.is_quiet(),
        "and the quiet device is below all of them"
    );
    assert!(d[0].total() > d[1].total());
    // The quiet row's *last* rates are whatever they were when the device
    // left — non-zero here. That is the point: the model keeps them, the sum
    // and the drawing must not use them.
    let (rd, wr) = c.model().totals();
    assert!(wr > 0.0);
    assert_eq!(
        rd,
        d.iter()
            .filter(|x| !x.live.is_quiet() && !x.is_partition())
            .map(|x| x.read_bps)
            .sum::<f64>(),
        "the total is over the devices still reporting, not over the rows"
    );
    let quiet_read: f64 = d
        .iter()
        .filter(|x| x.live.is_quiet())
        .map(|x| x.read_bps)
        .sum();
    assert!(
        rd < d.iter().map(|x| x.read_bps).sum::<f64>() || quiet_read == 0.0,
        "a device nobody is reporting is out of the sum (D68 §5)"
    );
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
    // `nvme0*` now matches the drive **and its partition** — the glob is a
    // name match and a partition's name begins with its parent's, which is
    // worth knowing and had no fixture before arc 16.
    let shown: Vec<&str> = c.model().drives.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(shown, ["nvme0n1", "nvme0n1p2"]);
    assert_eq!(c.model().hidden(), 3);
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(80, 12), false);
    let text = plain_text(&buf);
    assert!(text.contains("nvme0n1"));
    assert!(!text.contains("nvme1n1"));
    assert!(text.contains("(3 hidden)"), "{text}");
    // The partition draws as itself: its parent's temperature (same
    // controller), no model of its own, and no reads. A tile that rendered it
    // as `nvme0n1` would repeat the parent's row here.
    assert!(text.contains("nvme0n1p2"), "{text}");

    let mut c = Disk::new(Options {
        hide: vec!["nvme2*".into()],
        ..Options::default()
    });
    tick(&mut c, &store, TIER_TABLE);
    assert!(
        !c.model().drives.iter().any(|d| d.name == "nvme2n1"),
        "the hide glob is what this asserts"
    );
    assert_eq!(
        c.model().hidden(),
        1,
        "and it hid exactly the one it matched"
    );
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

/// **The defect D68 exists for, driven through the fixture.** A drive that is
/// unplugged keeps its series until retention evicts it, so before arc 17 its
/// row drew its last numbers with a green bullet and held its place in a
/// traffic sort — measured in a real terminal at `98M · 93% busy`.
///
/// `demo::DiskSynth`'s `sda` leaves at 45 s and stays gone for 135 s.
/// `store_without_sensors` ticks once a second, so 20 ticks is 20 s (plugged
/// in) and 60 ticks is 60 s (`sda` last reported at 44 s: fifteen batches
/// behind, well past the three the rule allows).
#[test]
fn a_device_that_stops_being_reported_draws_no_numbers_and_sinks() {
    let present = store_without_sensors(20); // 30 s — sda is plugged in
    let gone = store_without_sensors(60); // 90 s — sda left 60 s ago

    let row = |store: &Store| {
        let mut c = tile();
        tick(&mut c, store, TIER_TABLE);
        c.model()
            .drives
            .iter()
            .position(|d| d.name == "sda")
            .map(|i| (i, c.model().drives[i].clone()))
    };

    let (was_at, live) = row(&present).expect("sda is in the fixture while plugged in");
    assert!(!live.live.is_quiet(), "a reported device is not quiet");
    assert!(live.write_bps > 0.0, "and it is doing work");

    let (now_at, quiet) = row(&gone).expect("its series outlive the device, which is the bug");
    assert!(
        quiet.live.is_quiet(),
        "the source has moved on fifteen batches without sda and the tile still calls it live"
    );
    assert!(
        now_at > was_at,
        "a device with no measurement must not outrank one that has: sda was row {was_at} \
         while reporting and is row {now_at} after leaving"
    );
    assert_eq!(
        now_at,
        c_len(&gone) - 1,
        "and it sinks below every live row, not just one"
    );
}

fn c_len(store: &Store) -> usize {
    let mut c = tile();
    tick(&mut c, store, TIER_TABLE);
    c.model().drives.len()
}

/// The same, on screen: the row must draw dashes rather than a dimmed number,
/// because a dimmed `98M` still reads as a measurement and a `—` cannot
/// (D68 §5).
#[test]
fn a_quiet_row_draws_dashes_where_its_numbers_were() {
    let gone = store_without_sensors(60);
    let th = theme("modern");
    let mut c = tile();
    tick(&mut c, &gone, TIER_TABLE);
    // A rect that picks `table` and not `chart`: at a charting size the chart
    // prints its own series labels, and "sda" appears on a braille line that
    // is not the row under test. (The first version of this test found that
    // line and read as a failure of the fix.)
    let (tier, buf) = render_component(&mut c, &gone, &th, Size::new(50, 10), false);
    assert_eq!(c.tiers()[tier].name, "table");
    let text = plain_text(&buf);
    let line = text
        .lines()
        .find(|l| l.contains("sda"))
        .expect("the row is still drawn — it says the device was here");
    assert!(
        line.trim_start().starts_with('·'),
        "the bullet is the ghost `·`, not a live `●`: {line}"
    );
    // Every column after the name is a dash — asserted per cell, so removing
    // the dash from any one column (not only the busy one) fails here.
    let cells: Vec<&str> = line.split_whitespace().skip(2).collect();
    assert!(!cells.is_empty(), "the row has measured columns: {line}");
    assert!(
        cells.iter().all(|c| *c == "—"),
        "a device nobody is reporting draws a dash in every measured cell: {line}"
    );
    assert!(
        !line.contains('%'),
        "and no percentage, which is the cell that read `93%` for an unplugged drive: {line}"
    );
}

// ---- Arc 17 review (2026-09-18): the pieces the first pass left drawing a
// vanished drive as live, pinned with stores built by hand so that each
// assertion names the one thing it is about (a synth shifts every number).

/// A store fed by hand: `batches[i]` lists the `(name, read, write, busy)` the
/// disk source reports at second `i + 1`. A device missing from a later batch
/// is a device that left; its series stay in the store, which is the bug.
fn store_by_hand(batches: &[&[(&str, f64, f64, f64)]]) -> Store {
    let mut store = Store::default();
    for (i, devices) in batches.iter().enumerate() {
        let mut samples = Vec::new();
        for (name, read, write, busy) in devices.iter() {
            let id = |k: &gridwatch_store::Key<f64>| MetricId {
                name: k.id.name,
                label: Label::Name(Arc::from(*name)),
            };
            for (k, v) in [
                (&disk::READ_BPS, *read),
                (&disk::WRITE_BPS, *write),
                (&disk::BUSY_PCT, *busy),
            ] {
                samples.push(Sample {
                    id: id(k),
                    datum: Datum::Scalar(v),
                });
            }
        }
        store.apply(&Msg::Batch(Batch {
            source: disk::SOURCE,
            at: Ts((i as u64 + 1) * 1_000_000_000),
            samples,
        }));
    }
    store
}

/// `aaa` — alphabetically first, and the busiest thing on the machine when it
/// was last heard from — leaves after the first batch; `zzz` reports on. Both
/// sorts would put `aaa` first if liveness were ignored, so neither can pass
/// by accident. Six batches: `aaa` is five periods behind against a hold of
/// three.
fn aaa_leaves() -> Store {
    const BOTH: &[(&str, f64, f64, f64)] = &[("aaa", 50e6, 60e6, 93.0), ("zzz", 1e6, 2e6, 3.0)];
    const ONE: &[(&str, f64, f64, f64)] = &[("zzz", 1e6, 2e6, 3.0)];
    store_by_hand(&[BOTH, ONE, ONE, ONE, ONE, ONE])
}

/// D68 §5 and the brief's "done when" both name **both sorts**. The first pass
/// pinned only the default — the `Sort::Name` arm could lose its sink and
/// nothing failed (arc 17 review, lens E).
#[test]
fn a_quiet_drive_sinks_under_every_sort_the_tile_offers() {
    let store = aaa_leaves();
    let caps = CapSet::default();
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    let order =
        |c: &Disk| -> Vec<String> { c.model().drives.iter().map(|d| d.name.clone()).collect() };
    assert!(
        c.model().drives[1].live.is_quiet(),
        "the fixture must actually make `aaa` quiet"
    );
    assert_eq!(c.sort(), Sort::Traffic);
    assert_eq!(
        order(&c),
        ["zzz", "aaa"],
        "traffic: the frozen 110 MB/s must not outrank a live 3 MB/s"
    );
    c.on_key(key(KeyCode::Char('s')), &cx(&store, &caps));
    assert_eq!(c.sort(), Sort::Name);
    assert_eq!(
        order(&c),
        ["zzz", "aaa"],
        "name: alphabetical would put `aaa` first; a quiet row sinks under every sort"
    );
}

/// The total is over the devices still reporting. The first pass's version of
/// this compared the total with a filter that was the total's own filter, and
/// its second half was true because the fixture's removable reads 0 B/s — so
/// removing the exclusion changed nothing it could see. This fixture's quiet
/// drive reads **and** writes, so either half of a mistake shows.
#[test]
fn the_summed_rate_leaves_out_a_drive_nobody_is_reporting() {
    let store = aaa_leaves();
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(
        c.model().totals(),
        (1e6, 2e6),
        "zzz alone: aaa's frozen 50 MB/s read and 60 MB/s write are not being measured"
    );
}

/// The chip tiers name "the busiest" drive. A drive that left with the biggest
/// frozen number on the machine must not keep the title (lens C, F6: the
/// 8x3 head read `· 93%  sda`).
#[test]
fn the_small_tiers_do_not_name_a_vanished_drive_as_the_busiest() {
    let store = aaa_leaves();
    let mut c = tile();
    tick(&mut c, &store, TIER_RATES);
    assert_eq!(
        c.model().busiest().map(|d| d.name.as_str()),
        Some("zzz"),
        "the busiest of the drives still reporting"
    );
    let th = theme("modern");
    for (tier_name, size) in [("rates", Size::new(17, 8)), ("sparks", Size::new(30, 6))] {
        let (tier, buf) = render_component(&mut c, &store, &th, size, false);
        let text = plain_text(&buf);
        assert_eq!(c.tiers()[tier].name, tier_name);
        assert!(
            text.contains("zzz"),
            "{tier_name} names the live drive:\n{text}"
        );
        assert!(
            !text.contains("aaa") && !text.contains("93%"),
            "{tier_name} still names the vanished one or its frozen busy:\n{text}"
        );
    }
}

/// The zoomed `full` pane sat directly under a `—` table row and printed the
/// drive's last `busy 93% · q 2.2` and `r/s 0 · w/s 1493` as live — captured
/// in a real terminal (lens A, F1) and accepted into a snapshot hunk (lens E,
/// F7). The pane belongs to the drive it is about.
#[test]
fn the_zoomed_pane_of_a_quiet_drive_says_no_numbers() {
    let store = aaa_leaves();
    let th = theme("modern");
    let caps = CapSet::default();
    let mut c = tile();
    tick(&mut c, &store, TIER_FULL);
    // The pane describes the drive under the cursor; put it on the quiet one
    // (row two: it sank), as the person in the terminal did.
    c.on_key(key(KeyCode::Down), &cx(&store, &caps));
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    assert_eq!(c.tiers()[tier].name, "full");
    let text = plain_text(&buf);
    // The pane's `queue` line and the `scheduler` line under it. No digit may
    // follow `busy`, `r/s`, `w/s` or `q`. (A hand-built store has no
    // `disk.info`, so the header is the bare name; the two lines below it are
    // what a person reads.)
    let lines: Vec<&str> = text.lines().collect();
    let queue = lines
        .iter()
        .position(|l| l.trim_start().starts_with("queue") && l.contains("busy"))
        .unwrap_or_else(|| panic!("the pane's queue line is drawn:\n{text}"));
    for line in &lines[queue..(queue + 2).min(lines.len())] {
        for label in ["busy ", "r/s ", "w/s ", "q "] {
            if let Some(at) = line.find(label) {
                let next = line[at + label.len()..].chars().next();
                assert!(
                    !next.is_some_and(|c| c.is_ascii_digit()),
                    "a vanished drive's pane draws `{label}` with a number: {line}"
                );
            }
        }
    }
}

/// The age is not drawn at all (arc 17b). `gone 14s` was the store-time age at
/// the instant the source last spoke; once the source stalls, it froze beside
/// a `STALE` badge that kept counting wall time — two ages on one tile, on two
/// clocks (lens B, captured in a pty at 90 s). Knowing "the badge is down"
/// needs a clock this module deliberately does not read, so the brief's own
/// fallback applies: *draw no age at all*. `BACKLOG.md` keeps the clock
/// question for a Fable session.
#[test]
fn a_quiet_row_says_gone_and_never_how_long() {
    // One store stands for both cases on purpose: at a single instant the
    // store cannot tell a stalled source from an advancing one (that needs a
    // clock), which is precisely why the age cannot be drawn honestly.
    let store = store_without_sensors(60);
    let th = theme("modern");
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(90, 10), false);
    let text = plain_text(&buf);
    let line = text
        .lines()
        .find(|l| l.contains("sda"))
        .unwrap_or_else(|| panic!("the row is drawn:\n{text}"));
    assert!(line.contains("gone"), "the row says it is gone: {line}");
    let after = line.split("gone").nth(1).unwrap_or("");
    assert!(
        !after.trim_start().starts_with(|c: char| c.is_ascii_digit()),
        "a quiet row must not draw an age (D68 §5, brief seam 4): {line}"
    );
}
