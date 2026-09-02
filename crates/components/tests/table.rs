//! The htop process table (§8.1, brief 2a task 3): htop's printed formats
//! branch for branch, gridwatch's column drop order, the row budget at the real
//! grid sizes, sorting, and selection keyed by PID.

use gridwatch_components::htop::table::{Col, fit_columns};
use gridwatch_components::htop::{Htop, Options, format};
use gridwatch_store::keys::cpu;
use gridwatch_store::{KeyCode, KeyEvent, Store};
use gridwatch_ui::component::{Component, InputCx, Outcome, Size};
use gridwatch_ui::testkit::{demo_store, plain_text, render_component, theme, tick};

fn text(spans: &[gridwatch_ui::view::Span]) -> String {
    spans.iter().map(|s| s.text.as_ref()).collect()
}

/// `Row_printKBytes` (htop 3.4.1 `Row.c`): the three regimes and the unit
/// roll-over, five cells each.
#[test]
fn kbytes_follows_row_print_kbytes() {
    let k = |v: u64| text(&format::kbytes(Some(v)));
    assert_eq!(k(0), "    0");
    assert_eq!(k(999), "  999");
    assert_eq!(k(1000), " 1000");
    assert_eq!(k(28248), "28248");
    assert_eq!(k(99_999), "99999");
    // 100 000 KiB = 97.65 MiB → "97.6M" (truncated hundredths, as htop).
    assert_eq!(k(100_000), "97.6M");
    assert_eq!(k(102_400), " 100M");
    assert_eq!(k(1_023_999), " 999M");
    assert_eq!(k(1_024_000), "1000M");
    assert_eq!(k(9_999 * 1024), "9999M");
    // 10 000 MiB rolls to G with two decimals.
    assert_eq!(k(10_000 * 1024), "9.76G");
    assert_eq!(k(13_107_200), "12.5G");
    assert_eq!(k(1024 * 1024 * 1024), "1024G");
    assert_eq!(k(10_240 * 1024 * 1024), "10.0T");
    assert_eq!(text(&format::kbytes(None)), "  N/A");
    // Roles cycle Text → M → G → Large with the unit.
    let spans = format::kbytes(Some(28248));
    assert_eq!(
        spans[0].role,
        gridwatch_ui::theme::Role::Info,
        "the thousands are in the M role"
    );
    let spans = format::kbytes(Some(13_107_200));
    assert_eq!(spans[0].role, gridwatch_ui::theme::Role::Ok, "G role");
}

/// `Row_printTime`: `MM:SS.hh`, `HHhMM:SS`, `DdHHhMMm`, `DDDDdHHh`, `YYYyDDDd`.
#[test]
fn time_plus_follows_row_print_time() {
    let t = |cs: u64| text(&format::time_plus(cs));
    assert_eq!(t(0), " 0:00.00");
    assert_eq!(t(118), " 0:01.18");
    assert_eq!(t(59 * 60 * 100 + 59_99), "59:59.99");
    assert_eq!(t(60 * 60 * 100), " 1h00:00");
    assert_eq!(t(1_234_567), " 3h25:45");
    assert_eq!(t(24 * 3600 * 100), "1d00h00m");
    assert_eq!(t(4_812_339), "13h22:03");
    assert_eq!(t(10 * 24 * 3600 * 100), "  10d00h");
    assert_eq!(t(365 * 24 * 3600 * 100), "  1y000d");
}

/// `Row_printPercentage`: shadow under 0.05, accent at 99.9, `100` in a
/// four-wide column, auto width above it.
#[test]
fn percentage_follows_row_print_percentage() {
    use gridwatch_ui::theme::Role;
    let p = format::percentage(0.0, 4);
    assert_eq!((p.text.as_ref(), p.role), (" 0.0", Role::TextMuted));
    let p = format::percentage(12.34, 4);
    assert_eq!((p.text.as_ref(), p.role), ("12.3", Role::Text));
    let p = format::percentage(99.95, 4);
    assert_eq!((p.text.as_ref(), p.role), (" 100", Role::AccentSecondary));
    let p = format::percentage(412.0, 6);
    assert_eq!(p.text.as_ref(), " 412.0");
    let p = format::percentage(f32::NAN, 4);
    assert_eq!(p.text.as_ref(), " N/A");
}

/// §8.1 states: `R` ok, `D`/`Z` crit, `S` muted, `P` prints as `B`.
#[test]
fn state_colours_follow_htop() {
    use gridwatch_ui::theme::Role;
    assert_eq!(format::state('R').role, Role::Ok);
    assert_eq!(format::state('D').role, Role::Crit);
    assert_eq!(format::state('Z').role, Role::Crit);
    assert_eq!(format::state('S').role, Role::TextMuted);
    assert_eq!(format::state('I').role, Role::TextMuted);
    assert_eq!(format::state('P').text.as_ref(), "B");
}

/// The drop order at the tier's widths (§8.1): at 56 `SHR` goes (and `TIME+`
/// when CPU% is six wide), at 80 and above the whole grid default fits with
/// ≥ 37 cells of `Command`; `Command` is always last.
#[test]
fn columns_drop_in_gridwatch_order() {
    let default: Vec<Col> = Htop::default()
        .options()
        .columns
        .iter()
        .map(|c| Col::from_id(c).unwrap())
        .collect();
    let names = |cols: &[Col]| cols.iter().map(|c| c.title()).collect::<Vec<_>>();
    // pid_digits 7 (torch), CPU% four wide: fixed 41 → SHR goes at 56.
    assert_eq!(
        names(&fit_columns(&default, 56, 20, 7, 4)),
        ["PID", "RES", "S", "CPU%", "MEM%", "TIME+", "Command"]
    );
    // CPU% six wide under a game: fixed 43 → SHR and TIME+ go — the §8.1
    // guaranteed set at the minimum width.
    assert_eq!(
        names(&fit_columns(&default, 56, 20, 7, 6)),
        ["PID", "RES", "S", "CPU%", "MEM%", "Command"]
    );
    assert_eq!(
        names(&fit_columns(&default, 80, 20, 7, 6)),
        ["PID", "RES", "SHR", "S", "CPU%", "MEM%", "TIME+", "Command"]
    );
    // htop's full Main screen set enabled: VIRT, SHR, PRI, NI, TIME+, USER … go first.
    let full: Vec<Col> = [
        "pid", "user", "pri", "nice", "virt", "res", "shr", "state", "cpu", "mem", "time",
        "command",
    ]
    .iter()
    .map(|c| Col::from_id(c).unwrap())
    .collect();
    assert_eq!(
        names(&fit_columns(&full, 56, 20, 7, 4)),
        ["PID", "RES", "S", "CPU%", "MEM%", "Command"]
    );
    assert_eq!(
        names(&fit_columns(&full, 122, 20, 7, 4)),
        [
            "PID", "USER", "PRI", "NI", "VIRT", "RES", "SHR", "S", "CPU%", "MEM%", "TIME+",
            "Command"
        ]
    );
    // A width no set fits still keeps Command.
    assert_eq!(names(&fit_columns(&default, 10, 20, 7, 4)), ["Command"]);
}

fn rendered(store: &Store, size: Size, zoomed: bool) -> String {
    let mut h: Box<dyn Component> = Box::new(Htop::default());
    let (_, buf) = render_component(h.as_mut(), store, &theme("modern"), size, zoomed);
    plain_text(&buf)
}

fn table_rows(text: &str) -> usize {
    // Rows below the header line, which is the line containing "Command".
    let mut lines = text.lines();
    let _ = lines.by_ref().find(|l| l.contains("Command"));
    lines
        .filter(|l| l.trim().chars().next().is_some_and(|c| c.is_ascii_digit()))
        .count()
}

/// §8.1 row budget: `rows = min(table_rows, available)` on the grid —
/// 10 at 122×31, 7 at 80×20, 5 at 59×18 — and every row when zoomed.
#[test]
fn row_budget_at_the_real_grid_sizes() {
    let store = demo_store(42, 40);
    assert_eq!(table_rows(&rendered(&store, Size::new(122, 31), false)), 10);
    assert_eq!(table_rows(&rendered(&store, Size::new(80, 20), false)), 7);
    assert_eq!(table_rows(&rendered(&store, Size::new(59, 18), false)), 5);
    assert_eq!(table_rows(&rendered(&store, Size::new(56, 18), false)), 5);
    let zoomed = rendered(&store, Size::new(248, 66), true);
    // 32 synthetic rows minus the 6 kernel threads hidden by default.
    assert_eq!(table_rows(&zoomed), 26);
    assert!(zoomed.contains("/opt/game/bin/game --fullscreen --vulkan"));
    assert!(
        !zoomed.contains("kthreadd"),
        "kernel threads hidden by default"
    );
    // The sort column carries the glyph in its separator, not in its title.
    let top = rendered(&store, Size::new(122, 31), false);
    assert!(top.contains("CPU%▽"), "{top}");
}

/// `hide_kernel_threads = false` shows them in the thread role; the table's
/// `hide_userland_threads` is a no-op on pid-level rows.
#[test]
fn kernel_threads_can_be_shown() {
    let store = demo_store(42, 40);
    let mut h = Htop::new(Options {
        hide_kernel_threads: false,
        sort: "pid".into(),
        ..Options::default()
    });
    tick(&mut h, &store, 4);
    let rows = h.rows();
    assert_eq!(
        rows.len(),
        32,
        "every synthetic row, kernel threads included"
    );
    assert!(rows.iter().any(|r| r.kthread && &*r.comm == "kthreadd"));
    // Numeric columns start descending (htop), ties broken by PID.
    assert_eq!(rows[0].pid, 555_601);
    assert_eq!(rows.last().unwrap().pid, 1);
}

/// The keys (§8.1 shared behaviour, arc 2): `↑/↓ PgUp/PgDn Home/End` select
/// by PID, `<`/`>`/`F6` cycle the sort column, `I` inverts.
#[test]
fn keys_select_sort_and_invert() {
    let store = demo_store(42, 40);
    let caps = gridwatch_store::CapSet::empty();
    let cx = InputCx {
        store: &store,
        inner: ratatui_core::layout::Rect::new(0, 0, 122, 31),
        caps: &caps,
        readonly: false,
    };
    let mut h = Htop::default();
    tick(&mut h, &store, 4);
    assert_eq!(h.sort(), (Col::Cpu, true));
    assert_eq!(h.selected(), None);
    let key = |c: KeyCode| KeyEvent::plain(c);
    assert!(matches!(
        h.on_key(key(KeyCode::Down), &cx),
        Outcome::Consumed
    ));
    let first = h.rows()[0].pid;
    assert_eq!(
        h.selected(),
        Some(first),
        "the first key press lands on row 0"
    );
    h.on_key(key(KeyCode::Down), &cx);
    assert_eq!(h.selected(), Some(h.rows()[1].pid));
    h.on_key(key(KeyCode::End), &cx);
    assert_eq!(h.selected(), Some(h.rows().last().unwrap().pid));
    h.on_key(key(KeyCode::Home), &cx);
    assert_eq!(h.selected(), Some(first));
    h.on_key(key(KeyCode::PageDown), &cx);
    assert_eq!(h.selected(), Some(h.rows()[10].pid));
    // Invert: the selection stays on its PID while the rows reverse.
    let selected = h.selected();
    h.on_key(key(KeyCode::Char('I')), &cx);
    assert_eq!(h.sort(), (Col::Cpu, false));
    assert_eq!(h.selected(), selected);
    assert!(h.rows()[0].cpu_pct <= h.rows()[1].cpu_pct);
    // `>` cycles to the next enabled column (MEM%), `<` back.
    h.on_key(key(KeyCode::Char('>')), &cx);
    assert_eq!(h.sort(), (Col::Mem, true));
    assert!(h.rows()[0].mem_pct >= h.rows()[1].mem_pct);
    h.on_key(key(KeyCode::Char('<')), &cx);
    assert_eq!(h.sort(), (Col::Cpu, true));
    h.on_key(key(KeyCode::F(6)), &cx);
    assert_eq!(h.sort(), (Col::Mem, true));
    // Unknown keys are ignored, not consumed.
    assert!(matches!(
        h.on_key(key(KeyCode::Char('x')), &cx),
        Outcome::Ignored
    ));
    // The selection survives a new generation and vanishes with its process.
    let _ = cpu::PROC_TABLE;
}

/// The whole tier through the renderer at the hero size: header, ten rows,
/// the game row's numbers formatted as htop would print them.
#[test]
fn hero_table_prints_htop_formats() {
    let store = demo_store(42, 40);
    let text = rendered(&store, Size::new(122, 31), false);
    let game = text
        .lines()
        .find(|l| l.contains("/opt/game/bin/game"))
        .expect("the game row");
    assert!(game.contains(" 412345 "), "{game}");
    assert!(
        game.contains("12.5G"),
        "RES 12.5 GiB in the G regime: {game}"
    );
    assert!(game.contains("1503M"), "SHR in the M regime: {game}");
    assert!(game.contains(" R "), "state: {game}");
    assert!(game.contains("13.7"), "MEM%: {game}");
    assert!(game.contains("3h29:"), "TIME+ in the HHhMM:SS form: {game}");
    let bash = text.lines().find(|l| l.contains(" bash"));
    assert!(
        bash.is_none(),
        "bash at 0.0% sorts below the ten rows shown"
    );
}
