//! net component gate tests (§8, brief arc 7 seam 3): the tier per real
//! grid size, the hide list and the `a` key, the sort, the probe strip's
//! degraded note, the connection table's honest attribution, and the empty
//! tile.

use gridwatch_components::net::{Net, Options, Sort, TIER_TABLE};
use gridwatch_store::keys::net;
use gridwatch_store::{Detail, KeyCode, KeyEvent, Mods, Msg, Store, Ts};
use gridwatch_ui::component::{Component, InputCx, Outcome, Size, pick_tier};
use gridwatch_ui::testkit::{demo_store_at, plain_text, render_component, theme, tick};
use ratatui_core::layout::Rect;

fn tile() -> Net {
    Net::default()
}

/// The demo store carries the net synth's three interfaces.
fn store() -> Store {
    demo_store_at(42, 6, Detail::Table)
}

#[test]
fn net_tiers_match_the_real_grid_sizes() {
    let c = tile();
    let tier = |w, h, zoomed| {
        let (i, fallback) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        (c.tiers()[i].name, fallback)
    };
    assert_eq!(tier(17, 8, false), ("rates", false), "1x1 at 250x70");
    assert_eq!(tier(38, 8, false), ("sparks", false), "2x1 at 250x70");
    assert_eq!(tier(80, 20, false), ("conns", false), "4x2 at 250x70");
    assert_eq!(
        tier(122, 31, false),
        ("conns", false),
        "6x3: full is zoom-only"
    );
    assert_eq!(tier(48, 10, false), ("table", false));
    assert_eq!(tier(248, 66, true), ("full", false), "zoomed");
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

fn key(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        mods: Mods::NONE,
    }
}

#[test]
fn the_hide_list_keeps_the_noise_out_until_a_asks_for_it() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    let names: Vec<&str> = c.model().ifaces.iter().map(|i| i.name.as_str()).collect();
    assert!(names.contains(&"eno1"), "{names:?}");
    assert!(names.contains(&"wlp7s0"), "{names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("br-")),
        "a bridge is noise by default: {names:?}"
    );
    assert!(c.model().hidden() > 0);
    // `a` shows everything.
    assert!(matches!(
        c.on_key(key('a'), &cx(&store, &caps)),
        Outcome::Consumed
    ));
    assert!(c.show_all());
    let names: Vec<&str> = c.model().ifaces.iter().map(|i| i.name.as_str()).collect();
    assert!(names.iter().any(|n| n.starts_with("br-")), "{names:?}");
    assert_eq!(c.model().hidden(), 0);
    // An explicit filter wins over the default list.
    let mut only = Net::new(Options {
        interfaces: vec!["wl*".into()],
        ..Options::default()
    });
    tick(&mut only, &store, TIER_TABLE);
    assert_eq!(only.model().ifaces.len(), 1);
    assert_eq!(only.model().ifaces[0].name, "wlp7s0");
}

#[test]
fn the_sort_puts_the_busy_link_first_and_s_switches_it() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(c.sort(), Sort::Traffic);
    assert_eq!(
        c.model().ifaces[0].name,
        "eno1",
        "the interface carrying the traffic"
    );
    c.on_key(key('s'), &cx(&store, &caps));
    assert_eq!(c.sort(), Sort::Name);
    let names: Vec<&str> = c.model().ifaces.iter().map(|i| i.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "by name");
    // The primary interface is the default route's, whatever the sort.
    assert_eq!(c.model().primary().unwrap().name, "eno1");
}

#[test]
fn the_table_shows_states_rates_and_the_probe_strip() {
    let store = store();
    let th = theme("modern");
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(80, 20), false);
    assert_eq!(c.tiers()[tier].name, "conns");
    let text = plain_text(&buf);
    assert!(text.contains("iface"), "{text}");
    assert!(text.contains("eno1"), "{text}");
    assert!(text.contains("gateway"), "the probe strip: {text}");
    assert!(text.contains("ms"), "{text}");
    // The connection table names a process where the scan could read one,
    // and a uid where it could not.
    assert!(text.contains("firefox"), "{text}");
    assert!(text.contains("uid "), "an unattributed socket: {text}");
    // The footer says what is hidden and how to see it.
    assert!(text.contains("hidden") && text.contains("a all"), "{text}");
}

#[test]
fn the_full_tier_carries_the_route_and_the_probe_statistics() {
    let store = store();
    let th = theme("modern");
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(248, 66), true);
    assert_eq!(c.tiers()[tier].name, "full");
    let text = plain_text(&buf);
    assert!(text.contains("route"), "{text}");
    assert!(text.contains("192.168.100.1"), "the gateway: {text}");
    assert!(text.contains("dns"), "{text}");
    assert!(
        text.contains("public ip: off"),
        "opt-in, and it says so: {text}"
    );
    assert!(text.contains("mdev"), "the probe statistics: {text}");
    assert!(text.contains("mtu"), "the per-interface detail: {text}");
}

#[test]
fn an_empty_store_says_so_and_nothing_is_fabricated() {
    let th = theme("modern");
    let empty = Store::default();
    for size in [Size::new(17, 8), Size::new(80, 20), Size::new(248, 66)] {
        let mut c = tile();
        let (_, buf) = render_component(&mut c, &empty, &th, size, size.h > 60);
        let text = plain_text(&buf);
        assert!(
            text.contains("no interfaces") || text.contains("—"),
            "{text}"
        );
        assert!(!text.contains("Mb/s"), "no invented link: {text}");
    }
}

/// A source that reports zero traffic is not the same as one that reports
/// nothing: the rates read `0B`, not a dash.
#[test]
fn a_quiet_interface_reads_zero() {
    let mut store = Store::default();
    store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: net::SOURCE,
        at: Ts(1_000_000_000),
        samples: vec![
            gridwatch_store::Sample {
                id: net::RX_BPS.named(&std::sync::Arc::from("eth0")).id,
                datum: gridwatch_store::Datum::Scalar(0.0),
            },
            gridwatch_store::Sample {
                id: net::TX_BPS.named(&std::sync::Arc::from("eth0")).id,
                datum: gridwatch_store::Datum::Scalar(0.0),
            },
        ],
    }));
    let th = theme("modern");
    let mut c = tile();
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(17, 8), false);
    let text = plain_text(&buf);
    assert!(text.contains("0B"), "{text}");
    assert!(text.contains("↓") && text.contains("↑"), "{text}");
}

/// Arc 7a review: the tiers below `table` and the probe strip's own
/// branches had no test at all, and the connection table's assertions
/// passed while its local-address column was invisible.
#[test]
fn the_small_tiers_and_the_probe_strip_say_what_they_mean() {
    let store = store();
    let th = theme("modern");
    // `rates`: one interface's pair, with the arrows and a unit.
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(17, 8), false);
    assert_eq!(c.tiers()[tier].name, "rates");
    let text = plain_text(&buf);
    assert!(text.contains("eno1"), "{text}");
    assert!(text.contains("↓") && text.contains("↑"), "{text}");
    assert!(
        !text.contains("gateway"),
        "no probe strip this small: {text}"
    );
    // The three-row shape and the one-row fallback both draw the pair.
    let (_, short) = render_component(&mut tile(), &store, &th, Size::new(17, 4), false);
    let short = plain_text(&short);
    assert!(short.contains("↓") && short.contains("↑"), "{short}");
    // `sparks`: + the link's own words.
    let mut c = tile();
    let (tier, buf) = render_component(&mut c, &store, &th, Size::new(38, 8), false);
    assert_eq!(c.tiers()[tier].name, "sparks");
    let text = plain_text(&buf);
    assert!(text.contains("Gb/s") || text.contains("Mb/s"), "{text}");
}

/// The connection table draws **both** address columns. A renderer that
/// gave all the spare width to the last elastic column left `local` at
/// zero and drew the remote address under its header (arc 7a review).
#[test]
fn the_connection_table_shows_the_local_address_too() {
    let store = store();
    let th = theme("modern");
    let mut c = tile();
    let (_, buf) = render_component(&mut c, &store, &th, Size::new(90, 24), false);
    let text = plain_text(&buf);
    assert!(text.contains("local") && text.contains("remote"), "{text}");
    // The demo's established row: a private source address and a public
    // destination, each under its own heading.
    let row = text
        .lines()
        .find(|l| l.contains("140.82.112.4:443"))
        .unwrap_or_else(|| panic!("no established row: {text}"));
    assert!(
        row.contains("192.168."),
        "the local address is missing from the row: {row}"
    );
    assert!(
        row.find("192.168.").unwrap() < row.find("140.82.").unwrap(),
        "local must come before remote: {row}"
    );
}

/// Arc 7a review: `↑/↓` is advertised as "scroll" and only the connection
/// table moved. Given more interfaces than the tile has rows, the cursor
/// has to reach the ones below the fold.
#[test]
fn the_interface_table_scrolls_to_the_rows_below_the_fold() {
    let caps = gridwatch_store::CapSet::default();
    let th = theme("modern");
    // Thirty interfaces, none of them on the hide list — more than any
    // tile has rows.
    let mut store = Store::default();
    let names: Vec<String> = (0..30).map(|i| format!("eth{i:02}")).collect();
    store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: net::SOURCE,
        at: Ts(1_000_000_000),
        samples: names
            .iter()
            .enumerate()
            .flat_map(|(i, n)| {
                let name = std::sync::Arc::from(n.as_str());
                [
                    gridwatch_store::Sample {
                        id: net::RX_BPS.named(&name).id,
                        datum: gridwatch_store::Datum::Scalar(1000.0 - i as f64),
                    },
                    gridwatch_store::Sample {
                        id: net::TX_BPS.named(&name).id,
                        datum: gridwatch_store::Datum::Scalar(0.0),
                    },
                ]
            })
            .collect(),
    }));
    let mut c = tile();
    tick(&mut c, &store, TIER_TABLE);
    assert_eq!(c.model().ifaces.len(), 30);
    let last = names.last().expect("an interface").clone();
    let size = Size::new(80, 14);
    let (_, buf) = render_component(&mut c, &store, &th, size, false);
    let before = plain_text(&buf);
    assert!(
        !before.contains(&last),
        "the fixture must not fit: {before}"
    );
    for _ in 0..30 {
        c.on_key(
            KeyEvent {
                code: KeyCode::Down,
                mods: Mods::NONE,
            },
            &cx(&store, &caps),
        );
    }
    let (_, buf) = render_component(&mut c, &store, &th, size, false);
    let after = plain_text(&buf);
    assert!(
        after.contains(&last),
        "the last interface ({last}) never came into view: {after}"
    );
    // The cursor stops at the end rather than running past it.
    for _ in 0..50 {
        c.on_key(
            KeyEvent {
                code: KeyCode::Down,
                mods: Mods::NONE,
            },
            &cx(&store, &caps),
        );
    }
    let (_, buf) = render_component(&mut c, &store, &th, size, false);
    assert!(plain_text(&buf).contains(&last));
    // And `↑` walks back to the first.
    for _ in 0..60 {
        c.on_key(
            KeyEvent {
                code: KeyCode::Up,
                mods: Mods::NONE,
            },
            &cx(&store, &caps),
        );
    }
    let (_, buf) = render_component(&mut c, &store, &th, size, false);
    assert!(plain_text(&buf).contains("eth00"));
}
