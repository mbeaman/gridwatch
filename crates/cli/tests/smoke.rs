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
    let a = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "cells").unwrap();
    let b = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "cells").unwrap();
    assert_eq!(a, b);
    let a = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "ansi").unwrap();
    let b = gridwatch_app::shot(registry(), 42, 250, 70, "retrowave", 1, "ansi").unwrap();
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
    let frame = gridwatch_app::shot(registry(), 1, 250, 70, "retrowave", 1, "cells").unwrap();
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
            let out = gridwatch_app::shot(registry(), 7, w, h, theme, 1, "cells").unwrap();
            assert!(!out.is_empty(), "{theme} {w}x{h} empty");
        }
    }
}

/// Page 2 (Audio) renders too — chips for arc-5 kinds, and its 12x3 cpu tile
/// stays at the `meters` tier its placement pins (§4.6).
#[test]
fn shot_second_page() {
    let out = gridwatch_app::shot(registry(), 7, 250, 70, "retrowave", 2, "cells").unwrap();
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
    let big = plain(&gridwatch_app::shot(registry(), 1, 250, 70, "mono", 1, "cells").unwrap());
    assert!(big.contains("gridwatch"), "tab bar missing at 250x70");
    let dense = plain(&gridwatch_app::shot(registry(), 1, 120, 40, "mono", 1, "cells").unwrap());
    assert!(
        !dense.contains("gridwatch"),
        "tab bar visible in dense mode"
    );
}
