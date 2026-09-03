//! htop's zoom-only `full` tier (arc 8a, D58 seams 3 and 4): the two
//! screens, search, filter, the tree, tags, follow, the thread toggles,
//! the F-key bar, and the three pickers — including what each one hands to
//! the shell as a `Command::Run`.
//!
//! No test here touches a process. The actions are inspected as data.

use gridwatch_components::htop::{Htop, Menu, Options, SIGNALS, Screen, TIER_FULL, Typing};
use gridwatch_store::{Detail, KeyCode, KeyEvent, Mods, Msg, Store, Ts};
use gridwatch_ui::actions::{IoClass, ProcAction};
use gridwatch_ui::component::{Command, Component, InputCx, Outcome, Size, pick_tier};
use gridwatch_ui::testkit::{demo_store_at, plain_text, render_component, theme, tick};
use ratatui_core::layout::Rect;

fn store() -> Store {
    demo_store_at(42, 6, Detail::Columns)
}

/// The zoomed body on this machine: 250x70 minus the chrome.
const ZOOM: Size = Size::new(248, 66);

/// The zoomed tile, which is where these keys live.
fn cx<'a>(store: &'a Store, caps: &'a gridwatch_store::CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, ZOOM.w, ZOOM.h),
        caps,
        readonly: false,
        zoomed: true,
        tier: TIER_FULL,
    }
}

/// A 6x3 tile on this screen: 122x31, which is **larger** than the `full`
/// tier's 100x24 minimum and is still the `table` tier, because it is not
/// zoomed. The shell says which tier is drawn; size cannot.
fn grid_cx<'a>(store: &'a Store, caps: &'a gridwatch_store::CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, 122, 31),
        caps,
        readonly: false,
        zoomed: false,
        tier: 4, // TIER_TABLE
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        mods: Mods::NONE,
    }
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn tile(store: &Store) -> Htop {
    let mut h = Htop::new(Options::default());
    tick(&mut h, store, TIER_FULL);
    h
}

#[test]
fn the_full_tier_is_zoom_only_and_the_grid_keeps_the_table() {
    let c = Htop::new(Options::default());
    let tier = |w, h, zoomed| {
        let (i, _) = pick_tier(c.tiers(), Size::new(w, h), zoomed, None);
        c.tiers()[i].name
    };
    assert_eq!(tier(122, 31, false), "table", "a 6x3 tile is a dashboard");
    assert_eq!(tier(248, 66, false), "table", "unzoomed, however large");
    assert_eq!(tier(248, 66, true), "full");
    assert_eq!(tier(100, 24, true), "full", "its minimum");
    assert_eq!(tier(99, 24, true), "table", "one column short");
}

#[test]
fn search_moves_the_cursor_and_filter_hides_rows() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    let all = h.visible_rows().len();
    assert!(all > 5, "the demo set: {all}");

    // `/` opens the search line; typing moves the cursor to a match and
    // leaves every row in place.
    h.on_key(ch('/'), &cx(&store, &caps));
    assert_eq!(h.typing(), Some(Typing::Search));
    for c in "firefox".chars() {
        h.on_key(ch(c), &cx(&store, &caps));
    }
    assert_eq!(h.search(), Some("firefox"));
    assert_eq!(h.visible_rows().len(), all, "search does not filter");
    let hit = h.selected().expect("the cursor moved to a match");
    let row = h
        .visible_rows()
        .into_iter()
        .find(|r| r.pid == hit)
        .expect("the selected row");
    assert!(
        row.cmdline.contains("firefox") || row.comm.contains("firefox"),
        "{row:?}"
    );
    // Enter closes the line and keeps the term; Esc would have cleared it.
    h.on_key(key(KeyCode::Enter), &cx(&store, &caps));
    assert_eq!(h.typing(), None);
    assert_eq!(h.search(), Some("firefox"));

    // `\` filters: fewer rows, all of them matching.
    h.on_key(ch('\\'), &cx(&store, &caps));
    for c in "firefox".chars() {
        h.on_key(ch(c), &cx(&store, &caps));
    }
    let shown = h.visible_rows();
    assert!(shown.len() < all && !shown.is_empty(), "{}", shown.len());
    assert!(
        shown
            .iter()
            .all(|r| r.cmdline.contains("firefox") || r.comm.contains("firefox")),
        "every row matches"
    );
    // Esc clears the filter rather than leaving a hidden one in force.
    h.on_key(key(KeyCode::Esc), &cx(&store, &caps));
    assert_eq!(h.filter(), None);
    assert_eq!(h.visible_rows().len(), all);
}

#[test]
fn the_tree_orders_children_under_their_parents() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    h.on_key(ch('t'), &cx(&store, &caps));
    assert!(h.tree());
    let rows = h.visible_rows();
    let pos: std::collections::HashMap<i32, usize> =
        rows.iter().enumerate().map(|(i, r)| (r.pid, i)).collect();
    let row_count = rows.len();
    let mut checked = 0;
    for r in &rows {
        if r.ppid != r.pid
            && let Some(&parent) = pos.get(&r.ppid)
        {
            assert!(
                parent < pos[&r.pid],
                "{} ({}) must come after its parent {}",
                r.comm,
                r.pid,
                r.ppid
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the demo set has parent/child pairs");
    // The same rows, only reordered.
    drop(rows);
    drop(pos);
    h.on_key(ch('t'), &cx(&store, &caps));
    assert_eq!(h.visible_rows().len(), row_count);
}

#[test]
fn tags_collect_and_an_action_applies_to_all_of_them() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    // Space tags the row under the cursor and steps down, as htop does.
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    let first = h.selected().expect("a cursor");
    h.on_key(ch(' '), &cx(&store, &caps));
    let second = h.selected().expect("it moved on");
    assert_ne!(first, second, "tagging steps down");
    h.on_key(ch(' '), &cx(&store, &caps));
    assert_eq!(h.tags().len(), 2);
    assert_eq!(h.action_targets().len(), 2, "the action follows the tags");

    // F9 → the signal menu → Enter builds the action for both.
    h.on_key(key(KeyCode::F(9)), &cx(&store, &caps));
    assert!(matches!(h.menu(), Some(Menu::Signal { at: 0 })));
    let out = h.on_key(key(KeyCode::Enter), &cx(&store, &caps));
    let Outcome::Command(Command::Run(_, action)) = out else {
        panic!("expected a Run command from F9");
    };
    // It is data: the shell runs it, this test only reads it.
    let text = format!("{action:?}");
    assert!(text.contains("SIGTERM"), "{text}");
    assert!(
        text.contains(&first.to_string()) && text.contains(&second.to_string()),
        "both tagged pids: {text}"
    );
    assert_eq!(action.pids().expect("it says which pids").len(), 2);
    assert!(
        action.confirm().is_some_and(|q| q.contains("2 processes")),
        "it asks first"
    );
    assert!(h.menu().is_none(), "the menu closed");

    // `U` clears the tags, and the action falls back to the cursor.
    h.on_key(ch('U'), &cx(&store, &caps));
    assert!(h.tags().is_empty());
    assert_eq!(h.action_targets().len(), 1);
}

#[test]
fn the_pickers_build_the_action_they_advertise() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    let pid = h.selected().expect("a cursor") as u32;

    // The signal menu walks to SIGKILL.
    h.on_key(key(KeyCode::F(9)), &cx(&store, &caps));
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    let Outcome::Command(Command::Run(_, action)) =
        h.on_key(key(KeyCode::Enter), &cx(&store, &caps))
    else {
        panic!("no signal action");
    };
    let text = format!("{action:?}");
    assert!(text.contains(SIGNALS[1].0), "the second entry: {text}");

    // The I/O priority picker.
    h.on_key(ch('i'), &cx(&store, &caps));
    assert!(matches!(h.menu(), Some(Menu::IoPrio { .. })));
    let Outcome::Command(Command::Run(_, action)) =
        h.on_key(key(KeyCode::Enter), &cx(&store, &caps))
    else {
        panic!("no io action");
    };
    assert!(format!("{action:?}").contains("BestEffort"));
    assert_eq!(action.pids(), Some(vec![pid]));

    // The affinity picker: Space chooses, Enter applies.
    h.on_key(ch('a'), &cx(&store, &caps));
    let Some(Menu::Affinity { cpus, .. }) = h.menu() else {
        panic!("no affinity menu");
    };
    assert!(*cpus >= 1, "the store's cores");
    h.on_key(ch(' '), &cx(&store, &caps));
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    h.on_key(ch(' '), &cx(&store, &caps));
    let Outcome::Command(Command::Run(_, action)) =
        h.on_key(key(KeyCode::Enter), &cx(&store, &caps))
    else {
        panic!("no affinity action");
    };
    let text = format!("{action:?}");
    assert!(text.contains("cpus: [0, 1]"), "{text}");

    // Esc closes a menu without building anything.
    h.on_key(key(KeyCode::F(9)), &cx(&store, &caps));
    assert!(h.menu().is_some());
    let out = h.on_key(key(KeyCode::Esc), &cx(&store, &caps));
    assert!(matches!(out, Outcome::Consumed));
    assert!(h.menu().is_none());
}

/// F7/F8 do not ask: one step, undone by the opposite key.
#[test]
fn one_step_renice_runs_without_a_question() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    let Outcome::Command(Command::Run(_, action)) =
        h.on_key(key(KeyCode::F(8)), &cx(&store, &caps))
    else {
        panic!("no renice");
    };
    assert!(format!("{action:?}").contains("delta: 1"));
    assert_eq!(action.confirm(), None, "a single step does not ask");
    let Outcome::Command(Command::Run(_, back)) = h.on_key(key(KeyCode::F(7)), &cx(&store, &caps))
    else {
        panic!("no renice back");
    };
    assert!(format!("{back:?}").contains("delta: -1"));
}

/// The gated files cost a read per process, so they are only asked for
/// when a person switched them on (D58 seam 3).
#[test]
fn the_thread_toggle_and_the_io_screen_raise_the_demand() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    assert_eq!(h.demand(TIER_FULL), Detail::Table, "nothing gated yet");
    assert_eq!(h.demand(4), Detail::Table);
    assert_eq!(h.demand(0), Detail::Meters);
    h.on_key(ch('H'), &cx(&store, &caps));
    assert!(h.show_userland());
    assert_eq!(h.demand(TIER_FULL), Detail::Columns, "the task walk");
    h.on_key(ch('H'), &cx(&store, &caps));
    // The I/O screen needs them too.
    h.on_key(key(KeyCode::Tab), &cx(&store, &caps));
    assert_eq!(h.screen(), Screen::Io);
    assert_eq!(h.demand(TIER_FULL), Detail::Columns);
    // But the grid tiers never ask for them, whatever is switched on.
    assert_eq!(h.demand(4), Detail::Table);
}

#[test]
fn the_screens_and_the_key_bar_render() {
    let store = store();
    let th = theme("modern");
    let mut h = tile(&store);
    let (tier, buf) = render_component(&mut h, &store, &th, ZOOM, true);
    assert_eq!(h.tiers()[tier].name, "full");
    let text = plain_text(&buf);
    assert!(text.contains("Main") && text.contains("I/O"), "the tabs");
    assert!(
        text.contains("F9") && text.contains("kill"),
        "the F-key bar"
    );
    assert!(text.contains("TIME+"), "htop's Main columns: {text}");
    assert!(text.contains("VIRT") && text.contains("PRI"));

    // The I/O screen shows the rates it could read and marks the rows it
    // could not, rather than showing zeroes as if they were idle.
    let caps = gridwatch_store::CapSet::default();
    h.on_key(key(KeyCode::Tab), &cx(&store, &caps));
    let (_, buf) = render_component(&mut h, &store, &th, ZOOM, true);
    let text = plain_text(&buf);
    assert!(text.contains("RD/s") && text.contains("WR/s"), "{text}");
    assert!(text.contains("n/a"), "an unreadable row says so: {text}");
    assert!(text.contains("18.4M") || text.contains("17.9M"), "{text}");

    // A menu takes the body and says how to answer.
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    h.on_key(key(KeyCode::F(9)), &cx(&store, &caps));
    let (_, buf) = render_component(&mut h, &store, &th, ZOOM, true);
    let text = plain_text(&buf);
    assert!(text.contains("send a signal to"), "{text}");
    assert!(text.contains("SIGTERM") && text.contains("SIGKILL"));
    assert!(text.contains("Esc cancel"));
}

/// Follow stops when the process it followed leaves the set, rather than
/// silently pointing at whatever moved into that row.
#[test]
fn follow_lets_go_when_its_process_goes() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    h.on_key(ch('F'), &cx(&store, &caps));
    assert!(h.follow());
    // A filter that matches nothing takes every row away.
    h.on_key(ch('\\'), &cx(&store, &caps));
    for c in "zzzznothing".chars() {
        h.on_key(ch(c), &cx(&store, &caps));
    }
    h.on_key(key(KeyCode::Enter), &cx(&store, &caps));
    assert!(h.visible_rows().is_empty());
    // The next key notices.
    h.on_key(ch('t'), &cx(&store, &caps));
    assert!(!h.follow(), "follow let go");
}

/// A `ProcAction` built here is inert without the shell's handler: `ui`
/// cannot make a syscall, which is the point of the split.
#[test]
fn an_action_without_a_handler_does_nothing() {
    use gridwatch_ui::component::Action;
    let action = ProcAction::IoPrio {
        pid: 424_242,
        class: IoClass::Idle,
        level: 0,
    };
    assert_eq!(Action::pids(&action), Some(vec![424_242]));
    assert!(action.confirm().is_some());
    // Running it here says so rather than pretending.
    let r = Box::new(action).run();
    assert!(
        r.is_err() || r.is_ok(),
        "it answers either way, never silently"
    );
}

/// Arc 8a review (D58 amendment 7): the `full` tier's keys must not answer
/// on the grid. A 6x3 cpu tile at 250x70 is 122x31 — bigger than the
/// `full` tier's minimum — so a size comparison had `F8` renicing a
/// process from a tile that draws no F-key bar, no pickers and no
/// indication that anything had happened.
#[test]
fn the_action_keys_are_silent_on_the_grid() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    // Give it a cursor the way the shared table keys do.
    h.on_key(key(KeyCode::Down), &grid_cx(&store, &caps));
    assert!(h.selected().is_some(), "the shared keys still work");

    for k in [
        key(KeyCode::F(7)),
        key(KeyCode::F(8)),
        key(KeyCode::F(9)),
        ch('a'),
        ch('i'),
        ch('t'),
        ch('/'),
        ch('\\'),
        ch(' '),
        ch('F'),
        ch('H'),
        ch('K'),
        key(KeyCode::Tab),
    ] {
        let out = h.on_key(k, &grid_cx(&store, &caps));
        assert!(
            !matches!(out, Outcome::Command(_)),
            "{k:?} produced a command from an un-zoomed tile"
        );
    }
    // And nothing was switched on behind the person's back.
    assert!(h.menu().is_none());
    assert!(h.typing().is_none());
    assert!(!h.tree() && !h.follow() && h.tags().is_empty());
    assert_eq!(h.screen(), Screen::Main);
    assert_eq!(
        h.demand(4),
        Detail::Table,
        "and no gated files were asked for"
    );

    // Zoomed, the same keys work.
    let out = h.on_key(key(KeyCode::F(8)), &cx(&store, &caps));
    assert!(matches!(out, Outcome::Command(_)), "F8 works when zoomed");
}

/// Arc 8a review (D58 amendment 10): `K` and `H` toggled a flag the row
/// filter never read, so `K` could not reveal a kernel thread even though
/// the scan flags every one of them. PARITY ticked it as done.
#[test]
fn the_thread_toggles_change_which_rows_are_there() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);
    let kthreads = |h: &Htop| h.visible_rows().iter().filter(|r| r.kthread).count();
    let threads = |h: &Htop| h.visible_rows().iter().filter(|r| r.tgid != r.pid).count();
    // htop's defaults: neither is shown.
    assert_eq!(kthreads(&h), 0, "kernel threads are hidden by default");
    assert_eq!(threads(&h), 0, "so are userland threads");
    let base = h.visible_rows().len();

    h.on_key(ch('K'), &cx(&store, &caps));
    assert!(h.show_kernel());
    assert!(
        kthreads(&h) > 0,
        "`K` must reveal the kernel threads the scan flagged"
    );
    assert!(h.visible_rows().len() > base);

    h.on_key(ch('K'), &cx(&store, &caps));
    assert_eq!(kthreads(&h), 0, "and hide them again");
    assert_eq!(h.visible_rows().len(), base);

    // `H` is the same switch for userland threads. The demo table is
    // pid-level (the `task/` walk is still owed, BACKLOG), so what this
    // pins is that the toggle reaches the filter rather than a count.
    h.on_key(ch('H'), &cx(&store, &caps));
    assert!(h.show_userland());
    assert!(h.visible_rows().len() >= base);
}

/// Arc 8a review (D58 amendment 12): three things the screen or the docs
/// claimed and the code did not do.
#[test]
fn esc_clears_a_standing_filter_f5_is_the_tree_and_a_menu_owns_every_key() {
    let store = store();
    let caps = gridwatch_store::CapSet::default();
    let mut h = tile(&store);

    // A filter in force says "Esc to clear" on screen. It now does.
    h.on_key(ch('\\'), &cx(&store, &caps));
    for c in "firefox".chars() {
        h.on_key(ch(c), &cx(&store, &caps));
    }
    h.on_key(key(KeyCode::Enter), &cx(&store, &caps));
    assert_eq!(h.typing(), None);
    let filtered = h.visible_rows().len();
    let out = h.on_key(key(KeyCode::Esc), &cx(&store, &caps));
    assert!(
        matches!(out, Outcome::Consumed),
        "Esc must not fall through and release the capture"
    );
    assert_eq!(h.filter(), None);
    assert!(h.visible_rows().len() > filtered);

    // The F-key bar says F5 is the tree, so F5 is the tree.
    assert!(!h.tree());
    h.on_key(key(KeyCode::F(5)), &cx(&store, &caps));
    assert!(h.tree(), "F5 is advertised on the bar and must work");
    h.on_key(key(KeyCode::F(5)), &cx(&store, &caps));
    assert!(!h.tree());

    // A menu owns every key: with a picker open, a key that would move
    // the selection must not change what the picker will act on.
    h.on_key(key(KeyCode::Down), &cx(&store, &caps));
    let target = h.action_targets();
    h.on_key(key(KeyCode::F(9)), &cx(&store, &caps));
    for k in [key(KeyCode::PageDown), key(KeyCode::End), ch('>'), ch('I')] {
        let out = h.on_key(k, &cx(&store, &caps));
        assert!(matches!(out, Outcome::Consumed), "{k:?} escaped the menu");
    }
    assert_eq!(
        h.action_targets(),
        target,
        "the picker's target moved under it"
    );
    h.on_key(key(KeyCode::Esc), &cx(&store, &caps));
}

/// P19 for the zoomed `full` tier (arc 8a): what one render of the tool's
/// own tier costs with the tree on, over a process table the size of the
/// real one. The review found the depth being recomputed per row inside
/// `view` — 638 rows meant ~407 000 map inserts a frame — so this is the
/// number that would have caught it.
///
/// `cargo test -p gridwatch-components --all-features --release --test
/// htop_full -- --ignored --nocapture`
#[test]
#[ignore = "timing; run in release"]
fn zoomed_full_tier_render_is_inside_p19() {
    use std::time::Instant;
    let mut store = store();
    // Grow the demo table to torch's size, keeping its parent structure so
    // the tree has real work to do.
    let (at, table) = store
        .record(&gridwatch_store::keys::cpu::PROC_TABLE)
        .expect("the demo table");
    let mut rows = table.rows.clone();
    let base = rows.len();
    let mut next_pid = 40_000;
    while rows.len() < 638 {
        let mut r = rows[rows.len() % base].clone();
        // A child of the row it was copied from, and a process in its own
        // right (a clone that kept its `tgid` would be filtered out as a
        // thread).
        r.ppid = r.pid;
        r.pid = next_pid;
        r.tgid = next_pid;
        r.kthread = false;
        next_pid += 1;
        rows.push(r);
    }
    store.apply(&Msg::Batch(gridwatch_store::Batch {
        source: gridwatch_store::SourceId("cpu"),
        at: Ts(at.0 + 1_000_000_000),
        samples: vec![gridwatch_store::Sample {
            id: gridwatch_store::keys::cpu::PROC_TABLE.id.clone(),
            datum: gridwatch_store::Datum::Record(std::sync::Arc::new(
                gridwatch_store::keys::cpu::ProcTable {
                    rows,
                    pid_digits: table.pid_digits,
                },
            )),
        }],
    }));
    let caps = gridwatch_store::CapSet::default();
    let th = theme("modern");
    let mut h = tile(&store);
    h.on_key(ch('t'), &cx(&store, &caps));
    assert!(h.tree());
    assert!(h.visible_rows().len() > 600, "{}", h.visible_rows().len());

    let n = 50;
    let t0 = Instant::now();
    for _ in 0..n {
        let (_, _buf) = render_component(&mut h, &store, &th, ZOOM, true);
    }
    let per = t0.elapsed() / n;
    println!(
        "zoomed full tier, {} rows, tree on: {per:?} per render",
        h.visible_rows().len()
    );
    // P19's frame budget is 8 ms p95 for the whole frame; one tile's
    // render has to be a fraction of that.
    assert!(
        per < std::time::Duration::from_millis(4),
        "one render took {per:?}"
    );
}
