//! `gridwatch theme import` against real scheme files (D59 seam 2).
//!
//! The unit tests in `theme_import` cover the three parsers. This covers the
//! thing that actually matters to a person: a scheme someone else published
//! turns into a theme that **draws a frame**. Anything less and the import has
//! only moved the failure to first use.

use std::path::{Path, PathBuf};

use gridwatch_app::theme_import;
use gridwatch_ui::Registry;

/// The `bg` the imported theme declares, read back out of its own text — so
/// the assertion below is about the scheme's colour and not a constant.
fn imported_bg(toml: &str) -> String {
    toml.lines()
        .find_map(|l| l.strip_prefix("bg = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("the imported theme declares a background")
}

fn registry() -> Registry {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    reg
}

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/themes/import");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    assert!(
        out.len() >= 3,
        "expected one fixture per format in {}",
        dir.display()
    );
    out
}

/// Every shipped fixture imports, loads, and draws a real frame — and the
/// frame has the dashboard in it, not an empty screen.
#[test]
fn every_fixture_imports_and_draws() {
    let dir = std::env::temp_dir().join("gridwatch-theme-import-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for path in fixtures() {
        let who = path.file_name().unwrap().to_string_lossy().into_owned();
        let imported = theme_import::import(&path, None).unwrap_or_else(|e| panic!("{who}: {e}"));
        assert!(
            !imported.contrast.is_empty(),
            "{who}: no contrast report — the import must say what it made"
        );
        let out = dir.join(format!("{}.toml", imported.name));
        std::fs::write(&out, &imported.toml).unwrap();
        // The whole point: `--theme <path>` on the imported file draws.
        // 250x70: the configured mode, where the header and the key bar are
        // drawn too — at 120x40 the page is dense and the tab bar is hidden,
        // which is a layout fact rather than a theme one.
        let frame = gridwatch_app::shot(
            registry(),
            1,
            250,
            70,
            &out.to_string_lossy(),
            1,
            "cells",
            None,
        )
        .unwrap_or_else(|e| panic!("{who}: the imported theme does not render: {e}"));
        assert!(
            frame.contains("CPU") && frame.contains("gridwatch") && frame.contains("q quit"),
            "{who}: the imported theme drew an empty frame"
        );
        // And it drew in the scheme's own colours, not a fallback.
        let bg = imported_bg(&imported.toml);
        assert!(
            frame.contains(&bg),
            "{who}: the frame does not use the scheme's background {bg}"
        );
    }
}

/// An import is reproducible: the same scheme gives the same file. It is a
/// generated artefact, so it has to be diffable.
#[test]
fn importing_twice_gives_the_same_file() {
    for path in fixtures() {
        let a = theme_import::import(&path, None).expect("import");
        let b = theme_import::import(&path, None).expect("import");
        assert_eq!(a.toml, b.toml, "{}", path.display());
    }
}

/// `--name` wins over the scheme's own, and is slugged either way.
#[test]
fn the_name_can_be_overridden() {
    let path = fixtures()
        .into_iter()
        .find(|p| p.to_string_lossy().contains("gruvbox"))
        .expect("the base16 fixture");
    let default = theme_import::import(&path, None).expect("import");
    assert_eq!(
        default.name, "gruvbox-dark-medium",
        "the scheme names itself"
    );
    let named = theme_import::import(&path, Some("My Theme!")).expect("import");
    assert_eq!(named.name, "my-theme");
    assert!(named.toml.contains("name = \"my-theme\""));
}

/// A file that is not a scheme is refused by name, not guessed at.
#[test]
fn a_file_that_is_not_a_scheme_says_what_it_reads() {
    let dir = std::env::temp_dir().join("gridwatch-theme-import-bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shopping.txt");
    std::fs::write(&path, "milk\nbread\n").unwrap();
    let e = theme_import::import(&path, None).expect_err("not a scheme");
    let msg = e.to_string();
    assert!(
        msg.contains("alacritty") && msg.contains("wezterm") && msg.contains("base16"),
        "the refusal should name the three formats: {msg}"
    );
}
