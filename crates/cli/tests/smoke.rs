//! End-to-end headless smoke: the full registry through the shell (§12.5).

use gridwatch_ui::Registry;

fn registry() -> Registry {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    reg
}

/// Same seed, same size, same theme → byte-identical frames.
#[test]
fn shot_is_deterministic() {
    let a = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "cells", None).unwrap();
    let b = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "cells", None).unwrap();
    assert_eq!(a, b);
    let a = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "ansi", None).unwrap();
    let b = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "ansi", None).unwrap();
    assert_eq!(a, b);
}

/// Strip `[fg/bg/mods]` style tags: gradient titles emit one span per
/// character, so content assertions must run on the plain characters.
fn plain(cells: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in cells.chars() {
        match c {
            '[' => in_tag = true,
            ']' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// The frame carries the shell chrome and the arc-1a/1b tiles.
#[test]
fn shot_has_chrome_and_tiles() {
    let frame = gridwatch_app::shot(registry(), 1, 250, 70, "retrowave", 1, "cells", None).unwrap();
    let text = plain(&frame).to_lowercase();
    assert!(text.contains("gridwatch"), "tab bar missing");
    assert!(text.contains("sources"), "sources tile title missing");
    assert!(text.contains("state"), "sources table header missing");
    assert!(text.contains("network"), "the net tile's title missing");
    // Its 4x2 slot is wide enough for the interface table, so the rate
    // pair belongs to the smaller tiers: what proves the tile drew is the
    // interface it names and the probe strip under it.
    assert!(text.contains("eno1"), "no interface listed");
    assert!(text.contains("gateway"), "no probe strip");
    // The clock is borderless big-text; its glyph rows prove it rendered.
    assert!(
        frame.contains('▀') || frame.contains('█'),
        "big-text clock missing"
    );
    // Arc 1b: the htop tile at its `cores` tier — meters, CCD blocks, PSI.
    assert!(
        text.contains("ccd0") && text.contains("ccd1"),
        "cpu tile is not at `cores`"
    );
    assert!(text.contains("kthr;"), "the task line is missing");
    assert!(text.contains("psi cpu"), "the pressure row is missing");
    // Arc 2a: the top-N process table under the cores block.
    assert!(
        text.contains("time+") && text.contains("command"),
        "the process table header is missing"
    );
    assert!(
        text.contains("/opt/game/bin/game"),
        "the game row is missing"
    );
}

/// Every builtin theme renders the overview without panicking.
#[test]
fn shot_all_themes_all_sizes() {
    for theme in ["retrowave", "modern", "mono"] {
        for (w, h) in [(250u16, 70u16), (131, 37), (120, 40), (80, 24)] {
            let out = gridwatch_app::shot(registry(), 7, w, h, theme, 1, "cells", None).unwrap();
            assert!(!out.is_empty(), "{theme} {w}x{h} empty");
        }
    }
}

/// Page 2 (Audio) renders too — chips for arc-5 kinds, and its 12x3 cpu tile
/// stays at the `meters` tier its placement pins (§4.6).
#[test]
fn shot_second_page() {
    let out = gridwatch_app::shot(registry(), 7, 250, 70, "retrowave", 2, "cells", None).unwrap();
    assert!(!out.is_empty());
    let text = plain(&out).to_lowercase();
    assert!(text.contains("running"), "the cpu strip is missing");
    assert!(
        !text.contains("ccd0") && !text.contains("time+"),
        "`view = \"meters\"` grew into a richer tier"
    );
}

/// §6: dense mode hides the tab bar; configured mode shows it.
#[test]
fn dense_hides_tab_bar() {
    let big =
        plain(&gridwatch_app::shot(registry(), 1, 250, 70, "mono", 1, "cells", None).unwrap());
    assert!(big.contains("gridwatch"), "tab bar missing at 250x70");
    let dense =
        plain(&gridwatch_app::shot(registry(), 1, 120, 40, "mono", 1, "cells", None).unwrap());
    assert!(
        !dense.contains("gridwatch"),
        "tab bar visible in dense mode"
    );
}

// ---------------------------------------------------------------- D62: wide

/// Matt's Ptyxis size, once `MACHINE.md` records it: the same assertion runs
/// at it. `None` until then — nobody has measured the real terminal, and a
/// guessed size would pin nothing (D62 §4).
const MATT_TERMINAL: Option<(u16, u16)> = None;

/// The rows a tile's frame encloses, found by walking its border from the
/// title. The shell draws a titled box per placement; the focused tile's is
/// heavy (`┏ ┓ ┗ ┃`), the others' double (`╔ ╗ ╚ ║`).
fn tile_inner<'a>(rows: &'a [Vec<char>], title: &str) -> Vec<&'a [char]> {
    // The title is drawn immediately after the corner (`┏ CPU ━━`), so the
    // match must be anchored on one — `CPU` and `GPU` both appear in tile
    // *bodies* (`CPU [|||`, `GPU 2769MHz`, the ` GPU MEM` column head) and a
    // bare substring search would eventually find one of those instead.
    let needle: Vec<char> = format!(" {title} ").chars().collect();
    let corner = |c: char| c == '┏' || c == '╔';
    let (top, left) = rows
        .iter()
        .enumerate()
        .find_map(|(y, r)| {
            r.windows(needle.len())
                .position(|w| w == needle.as_slice())
                .filter(|x| *x > 0 && corner(r[x - 1]))
                .map(|x| (y, x - 1))
        })
        .unwrap_or_else(|| panic!("no tile titled {title:?} in the frame"));
    // In dense mode neighbouring tiles share border cells, so this tile's
    // top-right corner may have been overdrawn by the next tile's top-left
    // and its bottom-left by the tile below's top-left: the right edge is the
    // first corner or junction glyph after the title, and the bottom edge is
    // the first row where the left border stops being a vertical bar (arc 12
    // review — a 120×40 frame panicked or measured six tiles as one).
    let is_corner = |c: char| "┓╗┏╔┳╦┬╤┼╬┤╡┫╣".contains(c);
    let is_vertical = |c: char| matches!(c, '┃' | '║' | '│');
    let right = left
        + 1
        + rows[top][left + 1..]
            .iter()
            .position(|c| is_corner(*c))
            .unwrap_or_else(|| panic!("tile {title:?} has no top-right corner"));
    let bottom = top
        + 1
        + rows[top + 1..]
            .iter()
            .position(|r| !r.get(left).is_some_and(|c| is_vertical(*c)))
            .unwrap_or_else(|| panic!("tile {title:?} has no bottom edge"));
    rows[top + 1..bottom]
        .iter()
        .map(|r| &r[left + 1..right.min(r.len())])
        .collect()
}

/// Three numbers about a tile's inner rect: the fraction of cells that are
/// non-blank, the fraction of rows that carry anything, and the fraction of
/// columns that do. The cell fraction is D62's own measure; the row and column
/// coverage say whether a drawing reached the far side of the rect, which is
/// what the wide-terminal bug got wrong.
fn coverage(rows: &[Vec<char>], title: &str) -> (f64, f64, f64) {
    let inner = tile_inner(rows, title);
    let w = inner.iter().map(|r| r.len()).max().unwrap_or(0);
    let total: usize = inner.iter().map(|r| r.len()).sum();
    assert!(total > 0 && w > 0, "tile {title:?} has an empty inner rect");
    let filled: usize = inner
        .iter()
        .map(|r| r.iter().filter(|c| !c.is_whitespace()).count())
        .sum();
    let live_rows = inner
        .iter()
        .filter(|r| r.iter().any(|c| !c.is_whitespace()))
        .count();
    let live_cols = (0..w)
        .filter(|x| {
            inner
                .iter()
                .any(|r| r.get(*x).is_some_and(|c| !c.is_whitespace()))
        })
        .count();
    (
        filled as f64 / total as f64,
        live_rows as f64 / inner.len() as f64,
        live_cols as f64 / w as f64,
    )
}

fn frame_rows(w: u16, h: u16) -> Vec<Vec<char>> {
    let frame = gridwatch_app::shot(registry(), 1, w, h, "retrowave", 1, "ansi", None).unwrap();
    // `ansi` writes SGR sequences between cells; the cells themselves are the
    // characters left once those are dropped.
    let mut out = Vec::new();
    for line in frame.lines() {
        let mut row = Vec::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            row.push(c);
        }
        out.push(row);
    }
    out
}

/// D62's own report, as a test: at 480x135 the CPU and GPU tiles must fill
/// their rects, not draw a corner of them. Measured on this tree 2026-09-06,
/// with the frame before the arc-12 fixes beside it:
///
/// | 480x135 | cells | rows lit | cols lit |
/// |---------|-------|----------|----------|
/// | CPU before | 0.230 | 0.908 | 0.866 |
/// | CPU after  | 0.318 | 0.892 | 0.962 |
/// | GPU before | 0.100 | 0.323 | 1.000 |
/// | GPU after  | 0.107 | 0.554 | 1.000 |
///
/// The cell floors are 0.6x the measured value, as D62 asks; they are collapse
/// guards, not a re-proof of the fix. The **row** coverage is the number that
/// tells the two frames apart — the gpu chart band was clamped to eight rows
/// and left two thirds of the tile untouched — so its floor is 0.8x.
///
/// **Arc 15 (D65), measured 2026-09-09.** Every tile of the Overview, before
/// and after:
///
/// | 480x135 | cells before | after | rows before | after | cols before | after |
/// |---|---|---|---|---|---|---|
/// | CPU     | 0.333 | 0.333 | 0.892 | 0.892 | 0.996 | 0.996 |
/// | GPU     | 0.131 | **0.183** | 0.554 | **0.415** | 1.000 | 1.000 |
/// | PINS    | 0.314 | 0.314 | 0.585 | 0.585 | 1.000 | 1.000 |
/// | NETWORK | 0.058 | **0.152** | 0.268 | **0.415** | 0.561 | **1.000** |
/// | AUDIO   | 0.089 | 0.089 | 0.707 | 0.707 | 0.713 | 0.713 |
/// | SOURCES | 0.117 | 0.117 | 0.474 | 0.474 | 0.456 | 0.456 |
/// | SENSORS | 0.225 | **0.597** | 0.842 | 0.842 | 1.000 | 1.000 |
///
/// Three things in that table are worth more than the numbers.
///
/// **The gpu tile's row coverage went *down* while its cell coverage went up**,
/// and both are the same change: the 24-wide spec column lit sixteen rows down
/// the right-hand edge of a 46-row band, and it is now a two-row strip under a
/// chart that has the whole width. Fewer rows carry ink; 40 % more cells do.
/// Its row floor comes down with it, and the sentence saying why is the point
/// — D65 §9: **the cell fraction is a collapse guard, never a proof.**
///
/// **The `sensors` tile is the arc's headline and its 250×70 column coverage
/// did not move at all** (0.285 before and after — see the reference-size test
/// below), because the same number of columns carried ink; they were simply
/// eighty-seven cells apart. The defect these numbers were supposed to find
/// was invisible to them.
///
/// **The NETWORK number is fixture-shaped.** `demo::NetSynth` publishes five
/// connections and three interfaces while reporting `scanned: 103`; torch's
/// `/proc/net/{tcp,tcp6,udp,udp6}` hold 109 sockets and `/proc/net/dev` nine
/// interfaces, so on a real machine the connection band fills and the demo
/// leaves eighteen rows empty. This floor pins the synth as much as the tile
/// (D65's own note; fixing the synths is a `BACKLOG.md` item).
///
/// No floor for SOURCES, AUDIO or PINS: those numbers are content- or
/// construction-bounded and a floor would pin the demo synth rather than the
/// tile (AUDIO's ⅔ of columns is the bar-and-gap construction, SOURCES' rows
/// are its sources).
#[test]
fn a_wide_terminal_fills_its_tiles() {
    // (title, cell floor, lit-row floor, lit-column floor)
    let floors = [
        ("CPU", 0.19, 0.71, 0.77),
        ("GPU", 0.11, 0.33, 0.80),
        ("NETWORK", 0.09, 0.33, 0.80),
        ("SENSORS", 0.35, 0.67, 0.80),
    ];
    let mut sizes = vec![(480u16, 135u16)];
    sizes.extend(MATT_TERMINAL);
    for (w, h) in sizes {
        let rows = frame_rows(w, h);
        for (title, cells, r_floor, c_floor) in floors {
            let (c, lr, lc) = coverage(&rows, title);
            assert!(
                c >= cells && lr >= r_floor && lc >= c_floor,
                "the {title} tile at {w}x{h} covers {c:.3} of its cells, {lr:.3} of its rows and \
                 {lc:.3} of its columns (floors {cells}/{r_floor}/{c_floor}) — a drawing stopped \
                 scaling with its rect (D62, ARCHITECTURE §4.6)"
            );
        }
    }
}

/// Dense mode shares border cells between neighbours, so the finder must not
/// need a tile's own four corners (arc 12 review: it panicked on the GPU tile
/// at 120×40 and measured six tiles as one for the CPU tile). At 120×40 the
/// 6x3 inner rect is 59×18 (§8.1), 59×19 on the grid's top row, and one cell
/// narrower where the shared border is the neighbour's.
#[test]
fn the_tile_finder_survives_shared_borders() {
    let rows = frame_rows(120, 40);
    for title in ["CPU", "GPU"] {
        let inner = tile_inner(&rows, title);
        let w = inner.iter().map(|r| r.len()).max().unwrap_or(0);
        assert!(
            (18..=19).contains(&inner.len()) && (58..=59).contains(&w),
            "the {title} tile at 120x40 measures {w}x{} inner, expected 58–59 × 18–19",
            inner.len()
        );
    }
}

/// The same tiles at the reference size, so the floors are not a wide-only
/// accident. Measured 2026-09-06: CPU 0.347/0.939/0.984, GPU 0.291/0.818/1.000
/// (before the fixes: CPU 0.337/0.939/0.984, GPU 0.271/0.636/1.000).
///
/// **Arc 15, 2026-09-09.** CPU 0.347/0.939/0.984 (unmoved), GPU
/// 0.326 → 0.445 / 0.818 → 0.788 / 1.000, NETWORK 0.244 → 0.244 / 0.579 /
/// 0.889 → 0.840, SENSORS 0.204 → **0.716** / 1.000 / **0.285 → 0.870**.
///
/// The floors below are **not** re-fitted to the improved numbers (D65 trap
/// 1). Two of the movements are worth reading: `sensors` is where the arc
/// began, and at **480×135** its column coverage is 1.000 both before and
/// after: every column carried ink either way, with a temperature at column
/// 104 and the sensor it belongs to at column 17, so the metric scored the
/// broken frame perfectly. Its *cell* fraction is what moved (0.225 → 0.597
/// there, 0.204 → 0.716 at 250×70), which is why a review asks D65 §9's three
/// questions rather than reading one number — and NETWORK's column coverage went
/// **down**, from 0.889 to 0.840, because its connection table now ends at its
/// content. A falling number is what fixing a stretch looks like.
#[test]
fn the_reference_size_fills_its_tiles_too() {
    let rows = frame_rows(250, 70);
    for (title, cells, r_floor, c_floor) in [("CPU", 0.20, 0.75, 0.78), ("GPU", 0.17, 0.50, 0.80)] {
        let (c, lr, lc) = coverage(&rows, title);
        assert!(
            c >= cells && lr >= r_floor && lc >= c_floor,
            "the {title} tile at 250x70 covers {c:.3}/{lr:.3}/{lc:.3}, floors \
             {cells}/{r_floor}/{c_floor}"
        );
    }
}

/// The numbers the comments above record.
/// `cargo test -p gridwatch --test smoke -- --ignored measure_coverage`
#[test]
#[ignore = "diagnostic; prints the tile coverage the floors are set from"]
fn measure_coverage() {
    const TILES: [&str; 7] = [
        "CPU", "GPU", "PINS", "NETWORK", "AUDIO", "SOURCES", "SENSORS",
    ];
    for (w, h) in [(480u16, 135u16), (250, 70)] {
        let rows = frame_rows(w, h);
        for title in TILES {
            let (c, lr, lc) = coverage(&rows, title);
            println!("{w}x{h} {title}: cells {c:.3} rows {lr:.3} cols {lc:.3}");
        }
    }
}
