//! The global key table (§10, D59 seam 1).
//!
//! Before this existed the list of global keys lived in three places — the key
//! bar's hard-coded string, the `?` overlay's array beside it, and
//! `ARCHITECTURE.md` §10's prose — and nothing kept them equal. This is the
//! source of truth for all three: the shell draws the bar and the overlay from
//! it, and `gridwatch keybindings` writes `docs/KEYBINDINGS.md` from it, which
//! CI drift-checks like the metric catalogue.
//!
//! A *component's* keys are not here. They are already declared in its
//! `Manifest.keys`, which is the right place: a component that adds a key
//! documents it by declaring it. The generated document joins the two.

/// Where a binding applies. A person reading the help wants the grid keys and
/// the edit keys apart, and the key bar must not offer an edit key on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    /// Everywhere, edit mode and capture included.
    Always,
    /// The grid: not while editing, not while a component holds capture.
    Grid,
    /// Edit mode (`e`).
    Edit,
    /// Only meaningful while a `class = "showcase"` theme is drawing.
    Showcase,
}

impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Mode::Always => "everywhere",
            Mode::Grid => "the grid",
            Mode::Edit => "edit mode",
            Mode::Showcase => "showcase themes",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Binding {
    /// As a person would type it: `hjkl`, `^q`, `F12`, `1-9`.
    pub keys: &'static str,
    pub does: &'static str,
    pub mode: Mode,
    /// The label for the one-line key bar, or `None` to keep it out.
    ///
    /// A separate string rather than a truncation of `does`: the bar has room
    /// for a dozen entries and wants a word, while the help wants a sentence,
    /// and cutting the sentence at three words gives "zoom the focused".
    pub bar: Option<&'static str>,
}

const fn b(
    keys: &'static str,
    does: &'static str,
    mode: Mode,
    bar: Option<&'static str>,
) -> Binding {
    Binding {
        keys,
        does,
        mode,
        bar,
    }
}

/// Every global key, in the order a person should meet them.
pub const GLOBAL: &[Binding] = &[
    b(
        "q",
        "quit (when no component holds the keys)",
        Mode::Grid,
        Some("quit"),
    ),
    b("^q", "quit from anywhere", Mode::Always, None),
    b("?", "this help", Mode::Always, Some("help")),
    b("1-9", "go to a page by number", Mode::Always, None),
    b("[ ]", "previous / next page", Mode::Always, Some("pages")),
    b(
        "hjkl",
        "move focus (arrows work too)",
        Mode::Grid,
        Some("focus"),
    ),
    b(
        "Tab",
        "focus in reading order (Shift-Tab back)",
        Mode::Grid,
        None,
    ),
    b(
        "Enter",
        "give the keys to the focused tile (Esc takes them back)",
        Mode::Grid,
        Some("capture"),
    ),
    b("z", "zoom the focused tile", Mode::Grid, Some("zoom")),
    b(
        "d",
        "dense mode (override the size-derived one)",
        Mode::Grid,
        Some("dense"),
    ),
    b(
        "t",
        "cycle through the built-in themes",
        Mode::Always,
        Some("theme"),
    ),
    b(
        "T",
        "reload the current theme from its file",
        Mode::Always,
        Some("reload"),
    ),
    b(
        "space",
        "pause the sources (pins keeps sampling)",
        Mode::Always,
        Some("pause"),
    ),
    b("r", "start / stop recording a journal", Mode::Grid, None),
    b("a", "acknowledge the alert banner", Mode::Grid, Some("ack")),
    b(
        "A",
        "the alerts overlay: what is active, and the log",
        Mode::Grid,
        Some("alerts"),
    ),
    b(
        "S",
        "write a screenshot into the state directory",
        Mode::Grid,
        Some("shot"),
    ),
    b("e", "enter edit mode", Mode::Grid, Some("edit")),
    b(
        "F12",
        "the stats HUD (frame time, cells, bytes)",
        Mode::Always,
        Some("hud"),
    ),
    b("V", "re-light the whole page at once", Mode::Showcase, None),
    b(
        "L",
        "lock every tile lit (rain in the gutters only)",
        Mode::Showcase,
        None,
    ),
    b(
        "HJKL",
        "move the focused tile by one unit",
        Mode::Edit,
        None,
    ),
    b(
        "^h ^l ^j ^k",
        "narrow · widen · grow down · shrink up",
        Mode::Edit,
        None,
    ),
    b("s", "cycle the tile's footprint", Mode::Edit, None),
    b(
        "S then hjkl",
        "swap with the neighbour (Esc cancels)",
        Mode::Edit,
        None,
    ),
    b("a", "add a tile from the picker", Mode::Edit, None),
    b(
        "x",
        "remove the focused tile (Delete too)",
        Mode::Edit,
        None,
    ),
    b("u  ^r", "undo · redo", Mode::Edit, None),
    b("w", "save layout.toml", Mode::Edit, None),
    b(
        "Esc",
        "leave edit mode (asks if there are unsaved edits)",
        Mode::Edit,
        None,
    ),
];

/// The key bar for the grid, built to fit `width` by dropping **whole**
/// entries from the right.
///
/// It used to be one fixed string, which clipped mid-word below about 118
/// columns — an arc-1b review finding that stayed open until there was a list
/// to build it from. The first entry is always kept: a bar that says nothing
/// is worse than a bar that says one thing.
pub fn bar(width: u16) -> String {
    fit(&bar_entries(), width)
}

/// The bar's entries at full width, in table order.
fn bar_entries() -> Vec<String> {
    GLOBAL
        .iter()
        .filter(|b| matches!(b.mode, Mode::Always | Mode::Grid))
        .filter_map(|b| b.bar.map(|label| format!("{} {label}", b.keys)))
        .collect()
}

/// The bar for a captured tile: how to get out, then the tile's own keys,
/// dropped from the right the same way.
pub fn captured_bar(component: &[(&str, &str)], width: u16) -> String {
    let mut entries = vec!["Esc release".to_string()];
    if component.is_empty() {
        entries.push("component keys active".to_string());
    }
    entries.extend(component.iter().map(|(k, does)| format!("{k} {does}")));
    fit(&entries, width)
}

/// Join what fits, dropping whole entries from the right. Never returns an
/// empty string while there is at least one entry.
fn fit(entries: &[String], width: u16) -> String {
    const SEP: &str = " · ";
    // The shell draws the bar with one leading space.
    let room = usize::from(width).saturating_sub(1);
    let mut out = String::new();
    for e in entries {
        let extra = if out.is_empty() { 0 } else { SEP.len() };
        if !out.is_empty() && out.chars().count() + extra + e.chars().count() > room {
            break;
        }
        if !out.is_empty() {
            out.push_str(SEP);
        }
        out.push_str(e);
    }
    if out.is_empty() {
        out = entries.first().cloned().unwrap_or_default();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every binding says something, and no two rows of the same mode claim
    /// the same key — the drift this table exists to stop.
    #[test]
    fn the_table_is_well_formed() {
        for b in GLOBAL {
            assert!(!b.keys.is_empty(), "a binding with no key");
            assert!(!b.does.is_empty(), "{} does nothing", b.keys);
            assert!(
                b.does.chars().next().is_some_and(|c| c.is_lowercase()),
                "{}: `does` reads as a phrase, lower case: {:?}",
                b.keys,
                b.does
            );
        }
        for (i, a) in GLOBAL.iter().enumerate() {
            for other in &GLOBAL[i + 1..] {
                if a.keys == other.keys {
                    assert_ne!(
                        a.mode, other.mode,
                        "`{}` is bound twice in {:?}",
                        a.keys, a.mode
                    );
                }
            }
        }
    }

    /// The bar drops whole entries and never a partial word — the arc-1b
    /// finding, as a test at the widths that matter.
    #[test]
    fn the_bar_drops_whole_entries() {
        for width in [40u16, 80, 100, 118, 160, 250] {
            let line = bar(width);
            assert!(
                line.chars().count() <= usize::from(width).saturating_sub(1).max(1) || width < 20,
                "bar at {width} is {} chars: {line}",
                line.chars().count()
            );
            // Whatever survived is whole: every piece between separators is
            // an entry the table actually produces, not a prefix of one.
            let whole = bar_entries();
            for entry in line.split(" · ") {
                assert!(
                    whole.iter().any(|e| e == entry),
                    "bar at {width} ends in a fragment: {entry:?}"
                );
            }
            // And it is a prefix of the full bar: entries go from the right.
            assert!(
                bar(250).starts_with(&line) || width >= 250,
                "bar at {width} is not a prefix of the full bar:\n  {line}\n  {}",
                bar(250)
            );
        }
    }

    /// A narrow terminal still says one thing rather than nothing.
    #[test]
    fn a_very_narrow_bar_keeps_one_entry() {
        let line = bar(8);
        assert!(!line.is_empty());
        assert!(!line.contains(" · "), "{line}");
    }

    #[test]
    fn a_captured_bar_leads_with_the_way_out() {
        let line = captured_bar(&[("p", "pin the player"), ("x", "play")], 250);
        assert!(line.starts_with("Esc release"), "{line}");
        assert!(line.contains("p pin the player"), "{line}");
        let empty = captured_bar(&[], 250);
        assert!(empty.contains("component keys active"), "{empty}");
    }
}
