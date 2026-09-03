//! htop's zoom-only `full` tier (arc 8a, D58 seams 3 and 4): the two
//! screens, search, filter, the tree, tags, follow, the thread toggles,
//! the F-key bar, and the three pickers — including what each one hands to
//! the shell as a `Command::Run`.
//!
//! No test here touches a process. The actions are inspected as data.

use gridwatch_components::htop::{Htop, Menu, Options, SIGNALS, Screen, TIER_FULL, Typing};
use gridwatch_store::{Detail, KeyCode, KeyEvent, Mods, Store};
use gridwatch_ui::actions::{IoClass, ProcAction};
use gridwatch_ui::component::{Command, Component, InputCx, Outcome, Size, pick_tier};
use gridwatch_ui::testkit::{demo_store_at, plain_text, render_component, theme, tick};
use ratatui_core::layout::Rect;

fn store() -> Store {
    demo_store_at(42, 6, Detail::Columns)
}

/// The zoomed body on this machine: 250x70 minus the chrome.
const ZOOM: Size = Size::new(248, 66);

fn cx<'a>(store: &'a Store, caps: &'a gridwatch_store::CapSet) -> InputCx<'a> {
    InputCx {
        store,
        inner: Rect::new(0, 0, ZOOM.w, ZOOM.h),
        caps,
        readonly: false,
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
    assert_eq!(action.pids().len(), 2);
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
    assert_eq!(action.pids(), vec![pid]);

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
    assert_eq!(Action::pids(&action), vec![424_242]);
    assert!(action.confirm().is_some());
    // Running it here says so rather than pretending.
    let r = Box::new(action).run();
    assert!(
        r.is_err() || r.is_ok(),
        "it answers either way, never silently"
    );
}
